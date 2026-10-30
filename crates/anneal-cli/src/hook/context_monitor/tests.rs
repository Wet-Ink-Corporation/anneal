use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::run;
use crate::context::sessions::SessionFile;
use crate::context::usage::WindowSource;
use crate::context::{self, Saved, Snapshot, Source};
use crate::hook::HookEnv;

/// An anneal home whose status line has reported a reading for session `s1`.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        }
    }

    /// Saves a synthetic status-line reading of `used_tokens` of a 200,000-token window.
    fn report(&self, used_tokens: Option<f64>) {
        let path = context::in_home(self.dir.path(), context::USAGE_FILE);
        let mut file = SessionFile::<Snapshot>::load(&path, context::USAGE_KIND).expect("loads");
        file.set(
            "s1",
            Snapshot {
                used_tokens,
                window: 200_000,
                window_source: WindowSource::Default,
            },
        );
        file.save(&path).expect("saves");
    }

    /// Saves `used_tokens` of a 200,000-token window for session `s1` as `source` would,
    /// written at `at_ms`.
    fn save_from(&self, source: Source, used_tokens: f64, at_ms: u64) {
        let reading = Snapshot {
            used_tokens: Some(used_tokens),
            window: 200_000,
            window_source: WindowSource::Default,
        };
        context::save(
            self.dir.path(),
            "s1",
            Saved::stamped(reading, source, at_ms),
        )
        .expect("saves");
    }

    /// Runs the monitor for a `PostToolBatch` of session `s1`: its output, or `None` when silent.
    fn batch(&self) -> Option<Value> {
        self.monitor(r#"{"session_id":"s1","hook_event_name":"PostToolBatch","tool_calls":[]}"#)
    }

    fn monitor(&self, input: &str) -> Option<Value> {
        let home = self.dir.path().to_path_buf();
        let lookup = move |var: &str| (var == "ANNEAL_HOME").then(|| OsString::from(&home));
        let mut out = Vec::new();
        let env = HookEnv {
            stdin: input.as_bytes(),
            current_dir: || Ok(PathBuf::new()),
            lookup,
        };
        run(env, &mut out).expect("a Vec accepts every write");
        (!out.is_empty()).then(|| serde_json::from_slice(&out).expect("one JSON object"))
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// The `decision` of `output`.
fn decision(output: Option<&Value>) -> Option<&str> {
    output?.get("decision")?.as_str()
}

/// The `additionalContext` of `output`.
fn context_of(output: Option<&Value>) -> Option<&str> {
    output?
        .pointer("/hookSpecificOutput/additionalContext")?
        .as_str()
}

#[test]
fn with_no_status_line_reading_the_monitor_is_silent() {
    let home = Home::new();

    assert_eq!(home.batch(), None, "no usage file: silent");
    assert!(
        !context::in_home(home.path(), context::MONITOR_FILE).exists(),
        "and it records nothing"
    );
}

#[test]
fn another_session_s_reading_is_not_this_one_s() {
    let home = Home::new();
    home.report(Some(190_000.0));

    let output = home.monitor(r#"{"session_id":"s2","hook_event_name":"PostToolBatch"}"#);

    assert_eq!(output, None, "s2 has no reading: silent");
}

#[test]
fn below_60_percent_the_monitor_is_silent() {
    let home = Home::new();
    home.report(Some(119_999.0));

    assert_eq!(home.batch(), None, "under the warning");
}

#[test]
fn crossing_60_percent_warns_once_as_context() {
    let home = Home::new();
    home.report(Some(120_000.0));

    let first = home.batch();
    let second = home.batch();

    assert_eq!(decision(first.as_ref()), None, "a warning does not block");
    let warning = context_of(first.as_ref()).expect("a warning");
    assert!(
        warning.contains("120k tokens, 60% of the 200k-token auto-compact window"),
        "it says how full: {warning}"
    );
    assert_eq!(second, None, "the same crossing does not warn again");
}

#[test]
fn crossing_75_percent_stops_once_with_a_block() {
    let home = Home::new();
    home.report(Some(130_000.0));
    home.batch();
    home.report(Some(150_000.0));

    let stop = home.batch();
    let again = home.batch();

    assert_eq!(decision(stop.as_ref()), Some("block"), "75% stops the loop");
    let reason = stop
        .as_ref()
        .and_then(|output| output.get("reason"))
        .and_then(Value::as_str)
        .expect("a reason");
    assert!(reason.contains("/handoff"), "it says what to do: {reason}");
    assert_eq!(again, None, "the same crossing does not stop again");
}

#[test]
fn falling_below_a_threshold_rearms_it() {
    let home = Home::new();
    home.report(Some(160_000.0));
    let first = home.batch();
    home.report(Some(10_000.0));
    let compacted = home.batch();
    home.report(Some(160_000.0));

    let refilled = home.batch();

    assert_eq!(decision(first.as_ref()), Some("block"), "the first stop");
    assert_eq!(compacted, None, "after a compaction: silent");
    assert_eq!(
        decision(refilled.as_ref()),
        Some("block"),
        "a new crossing stops again"
    );
}

#[test]
fn a_reading_with_no_tokens_counts_as_below() {
    let home = Home::new();
    home.report(Some(125_000.0));
    home.batch();
    home.report(None);
    home.batch();
    home.report(Some(125_000.0));

    let output = home.batch();

    assert!(
        context_of(output.as_ref()).is_some(),
        "no tokens (after a compaction) re-arms the warning: {output:?}"
    );
}

#[test]
fn a_post_tool_use_input_is_answered_as_post_tool_use() {
    let home = Home::new();
    home.report(Some(120_000.0));

    let output = home.monitor(r#"{"session_id":"s1","hook_event_name":"PostToolUse"}"#);

    assert_eq!(
        output
            .as_ref()
            .and_then(|output| output.pointer("/hookSpecificOutput/hookEventName"))
            .and_then(Value::as_str),
        Some("PostToolUse"),
        "the event it was given: {output:?}"
    );
}

#[test]
fn a_state_file_that_cannot_be_written_fails_open_without_a_block() {
    let home = Home::new();
    home.report(Some(190_000.0));
    std::fs::create_dir(context::in_home(home.path(), context::MONITOR_FILE))
        .expect("a directory where the monitor's state file goes");

    let output = home.batch();

    assert_eq!(decision(output.as_ref()), None, "fails open: no block");
    let message = output
        .as_ref()
        .and_then(|output| output.get("systemMessage"))
        .and_then(Value::as_str)
        .expect("a system message");
    assert!(
        message.starts_with("ERROR context-monitor: ") && !message.contains('\n'),
        "one ERROR line: {message}"
    );
}

#[test]
fn an_unreadable_input_is_one_error_line_and_never_a_block() {
    let home = Home::new();
    home.report(Some(190_000.0));

    let output = home.monitor("not json");

    assert_eq!(decision(output.as_ref()), None, "fails open");
    let message = output
        .as_ref()
        .and_then(|output| output.get("systemMessage"))
        .and_then(Value::as_str)
        .expect("a system message");
    assert!(
        message.starts_with("ERROR context-monitor: ") && !message.contains('\n'),
        "one ERROR line: {message}"
    );
}

/// What fired at each batch: `"warn"`, `"stop"` or `"-"` for silence.
fn fired(output: Option<&Value>) -> &'static str {
    match (decision(output), context_of(output)) {
        (Some("block"), _) => "stop",
        (_, Some(_)) => "warn",
        _ => "-",
    }
}

/// The usage, in tokens of a 200,000-token window, at each batch of the crossing walk.
const WALK: [f64; 7] = [
    125_000.0, 125_000.0, 160_000.0, 160_000.0, 10_000.0, 130_000.0, 190_000.0,
];

/// The crossings `WALK` fires, one per batch.
const WALK_FIRES: [&str; 7] = ["warn", "-", "stop", "-", "-", "warn", "stop"];

#[test]
fn a_mod_snapshot_alone_fires_at_the_crossings_a_status_line_one_does() {
    let (status_line, from_mod) = (Home::new(), Home::new());

    let mut by_status_line = Vec::new();
    let mut by_mod = Vec::new();
    for (at_ms, used) in (1..).zip(WALK) {
        status_line.report(Some(used));
        by_status_line.push(fired(status_line.batch().as_ref()));
        from_mod.save_from(Source::Mod, used, at_ms);
        by_mod.push(fired(from_mod.batch().as_ref()));
    }

    assert_eq!(by_status_line, WALK_FIRES, "the status line's crossings");
    assert_eq!(by_mod, by_status_line, "the mod's are the same");
}

#[test]
fn the_fresher_of_the_two_snapshots_is_the_one_judged() {
    let home = Home::new();
    home.save_from(Source::Mod, 10_000.0, 100);
    home.save_from(Source::Statusline, 190_000.0, 200);

    let status_line_newer = home.batch();
    home.save_from(Source::Mod, 10_000.0, 300);
    let mod_newer = home.batch();
    home.save_from(Source::Statusline, 190_000.0, 400);
    let status_line_again = home.batch();

    assert_eq!(
        fired(status_line_newer.as_ref()),
        "stop",
        "the status line's 95% is newer than the mod's 5%"
    );
    assert_eq!(
        fired(mod_newer.as_ref()),
        "-",
        "the mod's newer 5%: below, so the stop re-arms"
    );
    assert_eq!(
        fired(status_line_again.as_ref()),
        "stop",
        "the status line is newer again: it stops again"
    );
}

#[test]
fn an_unstamped_status_line_snapshot_is_older_than_a_mod_one() {
    let home = Home::new();
    home.report(Some(190_000.0));
    home.save_from(Source::Mod, 10_000.0, 1);

    assert_eq!(home.batch(), None, "the mod's stamped 5% is judged: silent");
}
