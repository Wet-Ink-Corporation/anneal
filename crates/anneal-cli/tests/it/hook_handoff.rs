//! Surviving compaction through the built binary: the `handoff` hook writes `.continue-here`
//! on `PreCompact` and the next `SessionStart` injects it; with no status line configured the
//! context monitor is silent and the bootstrap gives a one-time setup hint.

use std::error::Error;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult = Result<(), Box<dyn Error>>;

/// What the bootstrap's setup hint says.
const HINT: &str = "the context monitor is off";

/// A committed repository, with the ledger and the anneal home outside it.
struct Repo {
    repo: TempDir,
    state: TempDir,
}

impl Repo {
    fn new() -> Result<Self, Box<dyn Error>> {
        let repo = tempfile::tempdir()?;
        git(repo.path(), &["init", "-q"]);
        std::fs::write(repo.path().join("README.md"), "scratch\n")?;
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "base"]);
        Ok(Self {
            repo,
            state: tempfile::tempdir()?,
        })
    }

    fn root(&self) -> &Path {
        self.repo.path()
    }

    /// Runs `anneal <args>` in the repository with `stdin`.
    fn run(&self, args: &[&str], stdin: &Value) -> Result<Output, Box<dyn Error>> {
        let mut child = anneal_in(self.root())
            .args(args)
            .env("ANNEAL_STORE", self.state.path().join("ledger.sqlite3"))
            .env("ANNEAL_HOME", self.state.path().join("home"))
            .env_remove("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().ok_or("stdin is piped")?;
        match input.write_all(&serde_json::to_vec(stdin)?) {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
            written => written?,
        }
        drop(input);
        let output = child.wait_with_output()?;
        assert_eq!(output.status.code(), Some(0), "exit 0: {output:?}");
        Ok(output)
    }

    /// The context the bootstrap injects at a `SessionStart` of `source`.
    fn session_start(&self, source: &str) -> Result<String, Box<dyn Error>> {
        let input = json!({
            "session_id": "s2",
            "cwd": self.root(),
            "hook_event_name": "SessionStart",
            "source": source,
        });
        let output = self.run(&["hook", "bootstrap"], &input)?;
        let json: Value = serde_json::from_slice(&output.stdout)?;
        Ok(json
            .pointer("/hookSpecificOutput/additionalContext")
            .and_then(Value::as_str)
            .ok_or("additionalContext")?
            .to_owned())
    }

    fn pre_compact(&self) -> Result<Output, Box<dyn Error>> {
        let input = json!({
            "session_id": "s1",
            "cwd": self.root(),
            "hook_event_name": "PreCompact",
            "trigger": "auto",
            "custom_instructions": null,
        });
        self.run(&["hook", "handoff"], &input)
    }

    fn monitor(&self) -> Result<Output, Box<dyn Error>> {
        let input = json!({
            "session_id": "s1",
            "cwd": self.root(),
            "hook_event_name": "PostToolBatch",
            "tool_calls": [],
        });
        self.run(&["hook", "context-monitor"], &input)
    }
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_precompact_handoff_is_injected_at_the_next_session_start() -> TestResult {
    let repo = Repo::new()?;

    let compacted = repo.pre_compact()?;
    let context = repo.session_start("compact")?;

    assert!(
        compacted.stdout.is_empty(),
        "the hook is silent: {compacted:?}"
    );
    let written = std::fs::read_to_string(repo.root().join(".continue-here"))?;
    assert!(
        written.contains("before an automatic compaction"),
        "written at the repository root: {written}"
    );
    assert!(
        context.starts_with("# anneal\n"),
        "the index first: {context}"
    );
    assert!(
        context.contains(&written),
        "the whole note is injected: {context}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_no_status_line_the_monitor_is_silent_and_the_bootstrap_hints_once() -> TestResult {
    let repo = Repo::new()?;

    let monitor = repo.monitor()?;
    let first = repo.session_start("startup")?;
    let second = repo.session_start("startup")?;

    assert!(monitor.stdout.is_empty(), "no reading: silent: {monitor:?}");
    assert!(first.contains(HINT), "the hint, once: {first}");
    assert!(!second.contains(HINT), "not again: {second}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_a_status_line_reading_there_is_no_hint_and_the_monitor_warns() -> TestResult {
    let repo = Repo::new()?;
    let status = json!({
        "session_id": "s1",
        "context_window": {"used_percentage": 65, "context_window_size": 200_000},
    });
    repo.run(&["statusline"], &status)?;

    let context = repo.session_start("startup")?;
    let monitor = repo.monitor()?;

    assert!(!context.contains(HINT), "configured: no hint: {context}");
    let warned: Value = serde_json::from_slice(&monitor.stdout)?;
    assert!(
        warned
            .pointer("/hookSpecificOutput/additionalContext")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("65%")),
        "65% of the window warns: {warned}"
    );
    Ok(())
}
