//! `anneal hook status`, the `SubagentStop` hook, through the built binary: every subagent's
//! last message is read for a status block and recorded as `subagent_status` with its timing
//! and tokens; a malformed block is a recorded violation, never a block or a crash.

use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult = Result<(), Box<dyn Error>>;

const GOOD: &str = "Finished.\n\nStatus: DONE\nGate: fmt ok | clippy ok | test ok (3 passed) | \
                    doc ok\nReview: approved\nChanged: src/a.rs (+3/-1)\nDecisions needed: none\n\
                    Not verified: none\n";

/// A repository with a ledger (`anneal mode off` creates it) and a subagent transcript.
struct Repo {
    dir: TempDir,
    transcript: PathBuf,
}

impl Repo {
    fn new(with_ledger: bool) -> Result<Self, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        git(dir.path(), &["init", "-q"]);
        if with_ledger {
            let mode = anneal_in(dir.path()).args(["mode", "off"]).output()?;
            assert_eq!(mode.status.code(), Some(0), "{mode:?}");
        }
        let transcript = dir.path().join(".git").join("agent-a1.jsonl");
        let lines = [
            json!({"type": "user", "timestamp": "2026-10-11T10:00:00Z",
                   "message": {"role": "user", "content": "go"}}),
            json!({"type": "assistant", "timestamp": "2026-10-11T10:01:30Z",
                   "message": {"id": "m1", "role": "assistant",
                               "content": [{"type": "text", "text": "from the transcript"}],
                               "usage": {"input_tokens": 5, "output_tokens": 50,
                                         "cache_read_input_tokens": 500,
                                         "cache_creation_input_tokens": 0}}}),
        ];
        let text: Vec<String> = lines.iter().map(Value::to_string).collect();
        std::fs::write(&transcript, text.join("\n") + "\n")?;
        Ok(Self { dir, transcript })
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn input(&self, message: Option<&str>) -> Value {
        let mut input = json!({
            "session_id": "s1",
            "transcript_path": self.root().join("main.jsonl"),
            "cwd": self.root(),
            "hook_event_name": "SubagentStop",
            "stop_hook_active": false,
            "agent_id": "a1",
            "agent_type": "temper:rust-implementer",
            "agent_transcript_path": self.transcript,
        });
        if let (Some(message), Some(object)) = (message, input.as_object_mut()) {
            object.insert("last_assistant_message".to_owned(), json!(message));
        }
        input
    }

    fn run(&self, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
        let mut child = anneal_in(self.root())
            .args(["hook", "status"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or("stdin is piped")?
            .write_all(stdin)?;
        Ok(child.wait_with_output()?)
    }

    /// Runs the hook on `message`, expecting exit 0 and no output.
    fn stop(&self, message: Option<&str>) -> TestResult {
        let output = self.run(&serde_json::to_vec(&self.input(message))?)?;
        assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "silent: {output:?}"
        );
        Ok(())
    }

    /// The recorded `subagent_status` envelopes.
    fn statuses(&self) -> Result<Vec<Value>, Box<dyn Error>> {
        let output = anneal_in(self.root())
            .args(["ledger", "query", "--type", "subagent_status"])
            .output()?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let listing: Value = serde_json::from_slice(&output.stdout)?;
        Ok(listing
            .pointer("/data/events")
            .and_then(Value::as_array)
            .ok_or("events")?
            .iter()
            .filter_map(|recorded| recorded.get("event").cloned())
            .collect())
    }
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_malformed_status_is_recorded_as_a_violation_not_a_crash() -> TestResult {
    let repo = Repo::new(true)?;

    repo.stop(Some("Status: WIP\nGate: red"))?;

    let statuses = repo.statuses()?;
    let [event] = statuses.as_slice() else {
        return Err(format!("one subagent_status, got {statuses:?}").into());
    };
    assert_eq!(event["status"], Value::Null, "no known status");
    let violation = event["violation"].as_str().ok_or("a violation")?;
    assert!(violation.contains("unknown status \"WIP\""), "{violation}");
    assert_eq!(
        event["missing"],
        json!(["Review", "Changed", "Decisions needed", "Not verified"]),
        "the absent sections"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_good_status_is_recorded_with_the_agent_timing_and_tokens() -> TestResult {
    let repo = Repo::new(true)?;

    repo.stop(Some(GOOD))?;

    let statuses = repo.statuses()?;
    let [event] = statuses.as_slice() else {
        return Err(format!("one subagent_status, got {statuses:?}").into());
    };
    assert_eq!(
        (
            &event["agent"],
            &event["agent_id"],
            &event["status"],
            &event["violation"],
            &event["duration_ms"],
            &event["tokens"],
            &event["session_id"],
        ),
        (
            &json!("temper:rust-implementer"),
            &json!("a1"),
            &json!("done"),
            &Value::Null,
            &json!(90_000),
            &json!({"input": 5, "output": 50, "cache_read": 500, "cache_creation": 0}),
            &json!("s1"),
        ),
        "status, timing and tokens from the transcript"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn without_last_assistant_message_the_transcripts_last_message_is_read() -> TestResult {
    let repo = Repo::new(true)?;

    repo.stop(None)?;

    let statuses = repo.statuses()?;
    let violation = statuses
        .first()
        .and_then(|event| event["violation"].as_str())
        .ok_or("a violation")?;
    assert!(
        violation.starts_with("no Status: line"),
        "the transcript's text has no status: {violation}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_no_ledger_nothing_is_recorded_and_none_is_created() -> TestResult {
    let repo = Repo::new(false)?;

    repo.stop(Some(GOOD))?;

    assert!(
        !repo.root().join(".git").join("anneal").exists(),
        "no ledger was created"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn unreadable_input_fails_open_with_one_error_line() -> TestResult {
    let repo = Repo::new(true)?;

    let output = repo.run(b"{\"hook_event_name\": 3")?;

    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let system = value["systemMessage"].as_str().ok_or("an ERROR line")?;
    assert!(system.starts_with("ERROR "), "{system}");
    assert_eq!(system.lines().count(), 1, "{system}");
    assert!(repo.statuses()?.is_empty(), "nothing recorded");
    Ok(())
}
