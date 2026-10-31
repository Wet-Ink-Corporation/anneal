use std::ffi::OsString;
use std::path::Path;

use clap::Parser;
use serde_json::Value;

use super::run;
use crate::cli::{Cli, Command};
use crate::context::sessions::SessionFile;
use crate::context::usage::WindowSource;
use crate::context::{self, Saved, Snapshot, Source, WriterEnv};
use crate::decide::clock::Clock;
use crate::error::CliError;
use crate::hook::clef_fake::Fixed;

/// What one run gave.
#[derive(Debug)]
enum Outcome {
    /// The exit code's value and the envelope.
    Written(u8, Value),
    /// The error.
    Failed(CliError),
}

impl Outcome {
    /// The code and envelope of a run that wrote.
    fn written(self) -> (u8, Value) {
        match self {
            Self::Written(code, envelope) => Some((code, envelope)),
            Self::Failed(_) => None,
        }
        .expect("written")
    }

    /// True when the run failed with the exit code `value`.
    fn failed_with(&self, value: u8) -> bool {
        matches!(self, Self::Failed(error) if error.code().value() == value)
    }
}

/// Runs `anneal snapshot write --json` on `input` with the anneal home `home` (none when
/// `None`) and the auto-compact window variable `window`, at [`Fixed`]'s time.
fn write(input: &str, home: Option<&Path>, window: Option<&str>) -> Outcome {
    let cli = Cli::try_parse_from(["anneal", "snapshot", "write", "--json"]).expect("parses");
    let args = match cli.command {
        Some(Command::Snapshot(args)) => Some(args),
        _ => None,
    }
    .expect("the snapshot command");
    let lookup = |var: &str| match var {
        "ANNEAL_HOME" => home.map(OsString::from),
        "CLAUDE_CODE_AUTO_COMPACT_WINDOW" => window.map(OsString::from),
        _ => None,
    };
    let env = WriterEnv {
        stdin: input.as_bytes(),
        lookup,
        clock: &Fixed,
    };
    let mut out = Vec::new();
    match run(&args, env, &mut out) {
        Ok(code) => Outcome::Written(
            code.value(),
            serde_json::from_slice(&out).expect("one JSON envelope"),
        ),
        Err(error) => Outcome::Failed(error),
    }
}

/// Session `session`'s snapshot in the usage file `file` in `home`.
fn saved(home: &Path, file: &str, session: &str) -> Option<Saved> {
    SessionFile::<Saved>::load(&context::in_home(home, file), context::USAGE_KIND)
        .expect("the usage file loads")
        .get(session)
        .copied()
}

/// The time [`Fixed`] reads.
fn now_ms() -> Option<u64> {
    Fixed.unix_millis().ok()
}

#[test]
fn a_snapshot_is_saved_as_the_mod_s_for_its_session() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let input = r#"{"session_id":"s1","used_tokens":120000,"used_percentage":60,
        "window":200000}"#;

    let (code, envelope) = write(input, Some(home.path()), None).written();

    assert_eq!(code, 0, "written");
    assert_eq!(
        saved(home.path(), context::MOD_USAGE_FILE, "s1"),
        Some(Saved {
            reading: Snapshot {
                used_tokens: Some(120_000.0),
                window: 200_000,
                window_source: WindowSource::Model,
            },
            source: Source::Mod,
            written_at_ms: now_ms(),
        }),
        "the mod's snapshot, stamped with the time"
    );
    assert!(
        !context::in_home(home.path(), context::USAGE_FILE).exists(),
        "the status line's file is the status line's"
    );
    assert_eq!(
        (
            envelope.get("ok"),
            envelope.get("command").and_then(Value::as_str),
            envelope.pointer("/data/source").and_then(Value::as_str),
        ),
        (
            Some(&Value::Bool(true)),
            Some("snapshot write"),
            Some("mod")
        ),
        "the envelope: {envelope}"
    );
}

#[test]
fn used_tokens_wins_over_a_used_percentage_that_disagrees() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let input = r#"{"session_id":"s1","used_tokens":50000,"used_percentage":60,
        "window":200000}"#;

    write(input, Some(home.path()), None).written();

    assert_eq!(
        saved(home.path(), context::MOD_USAGE_FILE, "s1").map(|saved| saved.reading.used_tokens),
        Some(Some(50_000.0)),
        "used_tokens (25%), not used_percentage's 60% of the window (120000)"
    );
}

#[test]
fn without_a_window_source_the_window_is_the_model_s_and_the_variable_applies() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let input = r#"{"session_id":"s1","used_tokens":100000,"window":1000000}"#;

    write(input, Some(home.path()), Some("400000")).written();

    assert_eq!(
        saved(home.path(), context::MOD_USAGE_FILE, "s1").map(|saved| saved.reading),
        Some(Snapshot {
            used_tokens: Some(100_000.0),
            window: 400_000,
            window_source: WindowSource::Env,
        }),
        "the auto-compact window, as the status line reads it"
    );
}

#[test]
fn a_saved_shape_with_its_window_source_is_kept_as_given() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let input = r#"{"session_id":"s1","used_percentage":50,"window":150000,
        "window_source":"env"}"#;

    write(input, Some(home.path()), Some("400000")).written();

    assert_eq!(
        saved(home.path(), context::MOD_USAGE_FILE, "s1").map(|saved| saved.reading),
        Some(Snapshot {
            used_tokens: Some(75_000.0),
            window: 150_000,
            window_source: WindowSource::Env,
        }),
        "the window as given, the tokens 50% of it"
    );
}

#[test]
fn no_tokens_and_no_percentage_saves_a_reading_with_no_tokens() {
    let home = tempfile::tempdir().expect("a temporary directory");

    write(
        r#"{"session_id":"s1","window":200000,"used_tokens":null}"#,
        Some(home.path()),
        None,
    )
    .written();

    assert_eq!(
        saved(home.path(), context::MOD_USAGE_FILE, "s1").map(|saved| saved.reading.used_tokens),
        Some(None),
        "nothing reported yet"
    );
}

#[test]
fn invalid_input_is_a_usage_error_that_writes_nothing() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let oversized = format!(r#"{{"session_id":"{}","window":1}}"#, "s".repeat(70_000));
    let cases = [
        "not json",
        r#"{"window":200000}"#,
        r#"{"session_id":"  ","window":200000}"#,
        r#"{"session_id":"s1"}"#,
        r#"{"session_id":"s1","window":0}"#,
        r#"{"session_id":"s1","window":200000,"used_tokens":-1}"#,
        r#"{"session_id":"s1","window":200000,"used_percentage":100.5}"#,
        r#"{"session_id":"s1","window":200000,"used_percentage":-1}"#,
        r#"{"session_id":"s1","window":200000,"window_source":"guess"}"#,
        r#"{"session_id":"s1","window":200000,"extra":1}"#,
        oversized.as_str(),
    ];

    for input in cases {
        let outcome = write(input, Some(home.path()), None);

        assert!(
            outcome.failed_with(2),
            "{input:.60}: a usage error, got {outcome:?}"
        );
    }
    assert!(
        !context::in_home(home.path(), context::MOD_USAGE_FILE).exists(),
        "nothing was written"
    );
}

#[test]
fn an_anneal_home_that_cannot_be_found_is_an_internal_error() {
    let outcome = write(r#"{"session_id":"s1","window":200000}"#, None, None);

    assert!(outcome.failed_with(70), "nowhere to save: {outcome:?}");
}

#[test]
fn a_usage_file_that_cannot_be_written_is_an_internal_error() {
    let home = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir(context::in_home(home.path(), context::MOD_USAGE_FILE))
        .expect("a directory where the file goes");

    let outcome = write(
        r#"{"session_id":"s1","window":200000}"#,
        Some(home.path()),
        None,
    );

    assert!(
        outcome.failed_with(70),
        "the file is not a file: {outcome:?}"
    );
}
