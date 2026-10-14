//! `anneal hook bootstrap` through the built binary: the index on every session start, live
//! state only when it changed, and failing open.

use std::error::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::Value;

use crate::support::{anneal_in, anneal_with_home};

type TestResult = Result<(), Box<dyn Error>>;

/// The first line of the index, which every injection starts with.
const INDEX_START: &str = "# anneal\n";

/// The heading of the live state section.
const LIVE_STATE: &str = "## Live state";

/// A scratch directory with a ledger path inside it (not created).
struct Scratch {
    dir: tempfile::TempDir,
    store: PathBuf,
}

impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let store = dir.path().join("anneal").join("ledger.sqlite3");
        Ok(Self { dir, store })
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Appends a `task_done` event for `task` to the ledger.
    fn append_task_done(&self, task: &str) -> TestResult {
        let event = format!(r#"{{"type":"task_done","task":"{task}"}}"#);
        let output = anneal_in(self.path())
            .args(["ledger", "--store"])
            .arg(&self.store)
            .args(["append", &event])
            .output()?;
        assert_eq!(output.status.code(), Some(0), "append: {output:?}");
        Ok(())
    }

    /// Runs `anneal hook bootstrap` with `stdin` as its input and the ledger at `store`.
    fn bootstrap(&self, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
        let mut child = anneal_in(self.path())
            .args(["hook", "bootstrap"])
            .env("ANNEAL_STORE", &self.store)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().ok_or("stdin is piped")?;
        match input.write_all(stdin) {
            // The hook stops reading at its cap and may exit before taking all of it.
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
            written => written?,
        }
        drop(input);
        Ok(child.wait_with_output()?)
    }

    /// The additional context injected for a `SessionStart` of `source` in `session`.
    fn context(&self, session: &str, source: &str) -> Result<String, Box<dyn Error>> {
        let input = serde_json::json!({
            "session_id": session,
            "transcript_path": "/dev/null",
            "cwd": self.path(),
            "hook_event_name": "SessionStart",
            "source": source,
        });
        injected(&self.bootstrap(&serde_json::to_vec(&input)?)?)
    }
}

/// Checks the hook exited 0 with a `SessionStart` output, and returns its additional context.
fn injected(output: &Output) -> Result<String, Box<dyn Error>> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "a hook fails open: {output:?}"
    );
    let json: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        json.pointer("/hookSpecificOutput/hookEventName"),
        Some(&Value::from("SessionStart")),
        "SessionStart output: {json}"
    );
    let context = json
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(Value::as_str)
        .ok_or("additionalContext is a string")?;
    Ok(context.to_owned())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn every_session_start_source_injects_the_index() -> TestResult {
    let scratch = Scratch::new()?;
    for source in ["startup", "clear", "compact", "resume"] {
        let context = scratch.context("s1", source)?;
        assert!(
            context.starts_with(INDEX_START),
            "{source}: the index comes first: {context}"
        );
        assert!(
            context.contains("- `grill`: "),
            "{source}: the index lists the entry skills"
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn live_state_comes_last_and_lists_the_ledger_tail() -> TestResult {
    let scratch = Scratch::new()?;
    scratch.append_task_done("T1")?;

    let context = scratch.context("s1", "startup")?;

    let (index, state) = context
        .split_once(LIVE_STATE)
        .ok_or("a live state section")?;
    assert!(index.starts_with(INDEX_START), "stable first: {context}");
    assert!(
        state.contains(" task_done task:T1\n"),
        "the tail names the event and its subject: {state}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn clearing_reinjects_the_index_but_not_an_unchanged_state() -> TestResult {
    // Kept under its original name; it checks the same thing as the test below. A real `/clear`
    // gets a new session id from Claude Code, so it does see the live state again.
    same_session_unchanged_state_gets_the_index_alone("clear")
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_same_session_with_an_unchanged_ledger_gets_the_index_but_not_the_live_state() -> TestResult {
    same_session_unchanged_state_gets_the_index_alone("compact")
}

/// A first start in session `s1` shows the live state; a second start of `source` in the same
/// session, with the ledger unchanged, shows the index and no live state.
fn same_session_unchanged_state_gets_the_index_alone(source: &str) -> TestResult {
    let scratch = Scratch::new()?;
    scratch.append_task_done("T1")?;
    let first = scratch.context("s1", "startup")?;
    assert!(first.contains(LIVE_STATE), "first start: {first}");

    let again = scratch.context("s1", source)?;

    assert!(again.starts_with(INDEX_START), "the index again: {again}");
    assert!(
        !again.contains(LIVE_STATE),
        "an unchanged state is not injected again: {again}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_changed_ledger_injects_the_live_state_again() -> TestResult {
    let scratch = Scratch::new()?;
    scratch.append_task_done("T1")?;
    scratch.context("s1", "startup")?;
    scratch.append_task_done("T2")?;

    let compacted = scratch.context("s1", "compact")?;

    assert!(
        compacted.contains(" task_done task:T2\n"),
        "the new event is injected: {compacted}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn another_session_gets_the_live_state_too() -> TestResult {
    let scratch = Scratch::new()?;
    scratch.append_task_done("T1")?;
    scratch.context("s1", "startup")?;

    let other = scratch.context("s2", "startup")?;

    assert!(other.contains(LIVE_STATE), "s2 never saw it: {other}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn without_a_ledger_there_is_no_live_state_and_none_is_created() -> TestResult {
    let scratch = Scratch::new()?;

    let context = scratch.context("s1", "startup")?;

    assert!(!context.contains(LIVE_STATE), "nothing to show: {context}");
    assert!(
        !scratch.store.exists(),
        "the bootstrap never creates a ledger"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_ledger_that_cannot_be_read_fails_open_with_a_note() -> TestResult {
    let scratch = Scratch::new()?;
    let parent = scratch.store.parent().ok_or("the store has a parent")?;
    std::fs::create_dir_all(parent)?;
    std::fs::write(&scratch.store, b"not a sqlite database, just text")?;

    let context = scratch.context("s1", "startup")?;

    assert!(
        context.starts_with(INDEX_START),
        "the index still: {context}"
    );
    let state = context
        .split_once(LIVE_STATE)
        .map(|(_, state)| state)
        .ok_or("a live state section with the note")?;
    assert!(
        state.contains("The ledger could not be read: "),
        "one line says why: {state}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn input_that_is_not_json_still_injects_the_index() -> TestResult {
    let scratch = Scratch::new()?;

    let context = injected(&scratch.bootstrap(b"this is not json")?)?;

    assert!(context.starts_with(INDEX_START), "fail open: {context}");
    assert!(
        context.contains("anneal could not read the hook input"),
        "a short note says so: {context}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn input_over_the_cap_still_injects_the_index() -> TestResult {
    let scratch = Scratch::new()?;
    let huge = vec![b' '; 1024 * 1024];

    let context = injected(&scratch.bootstrap(&huge)?)?;

    assert!(context.starts_with(INDEX_START), "fail open: {context}");
    assert!(
        context.contains("larger than"),
        "the note names the cap: {context}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_closed_stdout_still_exits_0_with_one_error_line() -> TestResult {
    let scratch = Scratch::new()?;
    let mut child = anneal_in(scratch.path())
        .args(["hook", "bootstrap"])
        .env("ANNEAL_STORE", &scratch.store)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Close the read end of stdout before the hook writes anything.
    drop(child.stdout.take());
    let mut input = child.stdin.take().ok_or("stdin is piped")?;
    input.write_all(b"{}")?;
    drop(input);

    let output = child.wait_with_output()?;

    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    let err = String::from_utf8(output.stderr)?;
    assert!(
        err.starts_with("ERROR writing output") && err.lines().count() == 1,
        "one ERROR line says why: {err}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn lint_skills_accepts_the_committed_index() -> TestResult {
    // `lint skills` runs its index rule only when it lints at least one skill, and
    // plugin/skills has none yet, so lint a passing fixture skill with the shipped index.
    let (code, flagged) = lint_index("docs/bootstrap-index.md")?;
    assert_eq!(code, Some(0), "lint skills exits 0");
    assert!(!flagged, "no bootstrap-index finding for the shipped index");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn lint_skills_rejects_an_index_over_budget_in_the_same_setup() -> TestResult {
    // The control for the test above: the same setup does run the index rule.
    let (code, flagged) = lint_index(
        "crates/anneal-cli/tests/fixtures/lint-skills/rule-04-bootstrap-index/fail/index.md",
    )?;
    assert_eq!(code, Some(1), "an error finding exits 1");
    assert!(flagged, "the oversized index is flagged");
    Ok(())
}

/// Runs `anneal lint skills <passing fixture skills> --index <index>` from the repository root:
/// the exit code, and whether a `bootstrap-index` finding was reported.
fn lint_index(index: &str) -> Result<(Option<i32>, bool), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let skills = "crates/anneal-cli/tests/fixtures/lint-skills/rule-04-bootstrap-index/pass/skills";
    let home = tempfile::tempdir()?;
    let output = anneal_with_home(&root, home.path())
        .args(["lint", "skills", skills, "--index", index])
        .output()?;
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    let flagged = envelope
        .pointer("/data/details")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|detail| detail.get("checker") == Some(&Value::from("bootstrap-index")));
    Ok((output.status.code(), flagged))
}
