//! `anneal hook guard` through the built binary: a deny decision with one `ERROR` line and a
//! `gate_blocked` event, silence on pass, and failing closed on bad input, a usage error or an
//! unwritable stdout.

use std::error::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

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

    /// Runs `anneal hook guard --fail-closed` with `stdin`, the ledger at `store`.
    fn guard(&self, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
        let mut child = anneal_in(self.path())
            .args(["hook", "guard", "--fail-closed"])
            .env("ANNEAL_STORE", &self.store)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().ok_or("stdin is piped")?;
        match input.write_all(stdin) {
            // The guard stops reading at its cap and may exit before taking all of it.
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
            written => written?,
        }
        drop(input);
        Ok(child.wait_with_output()?)
    }

    /// The `PreToolUse` input for a Bash call of `command` in this directory.
    fn bash(&self, command: &str) -> Result<Vec<u8>, Box<dyn Error>> {
        Ok(serde_json::to_vec(&json!({
            "session_id": "s-it",
            "transcript_path": "/dev/null",
            "cwd": self.path(),
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command, "description": "test"},
            "tool_use_id": "toolu_1",
        }))?)
    }

    /// The `gate_blocked` events in the ledger, through `anneal ledger query`.
    fn blocked(&self) -> Result<Vec<Value>, Box<dyn Error>> {
        let output = anneal_in(self.path())
            .args(["ledger", "--store"])
            .arg(&self.store)
            .args(["query", "--type", "gate_blocked"])
            .output()?;
        assert_eq!(output.status.code(), Some(0), "query: {output:?}");
        let envelope: Value = serde_json::from_slice(&output.stdout)?;
        Ok(envelope
            .pointer("/data/events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }
}

/// The deny reason in `output`, after checking it is exactly one deny decision, exit 0, and
/// nothing on stderr.
fn deny_reason(output: &Output) -> Result<String, Box<dyn Error>> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "a decision exits 0: {output:?}"
    );
    assert!(output.stderr.is_empty(), "nothing on stderr: {output:?}");
    let decision: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        decision.pointer("/hookSpecificOutput/permissionDecision"),
        Some(&Value::from("deny")),
        "a deny decision: {decision}"
    );
    assert_eq!(
        decision.pointer("/hookSpecificOutput/hookEventName"),
        Some(&Value::from("PreToolUse")),
        "for PreToolUse: {decision}"
    );
    let reason = decision
        .pointer("/hookSpecificOutput/permissionDecisionReason")
        .and_then(Value::as_str)
        .ok_or("a reason")?;
    assert!(
        reason.starts_with("ERROR ") && !reason.contains('\n'),
        "one ERROR line: {reason}"
    );
    Ok(reason.to_owned())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_denied_command_gets_a_deny_decision_and_a_gate_blocked_event() -> TestResult {
    let scratch = Scratch::new()?;

    let output = scratch.guard(&scratch.bash("cd x && git push --force origin main")?)?;

    let reason = deny_reason(&output)?;
    assert!(
        reason.starts_with("ERROR guard force-push: force push to protected branch main"),
        "names the rule and the branch: {reason}"
    );
    let events = scratch.blocked()?;
    let [event] = events.as_slice() else {
        return Err(format!("one gate_blocked event, got {events:?}").into());
    };
    assert_eq!(
        event.pointer("/event/point"),
        Some(&Value::from("guard.force-push")),
        "{event}"
    );
    assert_eq!(
        event.pointer("/event/session_id"),
        Some(&Value::from("s-it")),
        "{event}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_allowed_command_is_silent_and_records_nothing() -> TestResult {
    let scratch = Scratch::new()?;

    let output = scratch.guard(&scratch.bash("git push -u origin feature && rm -rf /tmp/x")?)?;

    assert_eq!(output.status.code(), Some(0), "allowed: {output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "silent on pass: {output:?}"
    );
    assert!(!scratch.store.exists(), "no ledger is created for a pass");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn malformed_input_is_denied() -> TestResult {
    let scratch = Scratch::new()?;

    let output = scratch.guard(b"{\"tool_name\": \"Bash\", \"tool_input\": ")?;

    let reason = deny_reason(&output)?;
    assert!(
        reason.starts_with("ERROR guard bad-input: the hook input cannot be read"),
        "fail closed: {reason}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn input_over_the_cap_is_denied() -> TestResult {
    let scratch = Scratch::new()?;
    let huge = vec![b' '; 2 * 1024 * 1024];

    let output = scratch.guard(&huge)?;

    let reason = deny_reason(&output)?;
    assert!(reason.contains("larger than"), "fail closed: {reason}");
    Ok(())
}

/// Runs `anneal <args>` with `stdin` and the read end of its stdout closed before it writes.
fn with_closed_stdout(
    scratch: &Scratch,
    args: &[&str],
    stdin: &[u8],
) -> Result<Output, Box<dyn Error>> {
    let mut child = anneal_in(scratch.path())
        .args(args)
        .env("ANNEAL_STORE", &scratch.store)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    drop(child.stdout.take());
    let mut input = child.stdin.take().ok_or("stdin is piped")?;
    input.write_all(stdin)?;
    drop(input);
    Ok(child.wait_with_output()?)
}

/// Asserts `output` exited 2 with exactly one `ERROR writing output` line on stderr.
fn assert_blocked_with_one_error_line(output: Output) -> TestResult {
    assert_eq!(
        output.status.code(),
        Some(2),
        "output that cannot be delivered blocks: {output:?}"
    );
    let err = String::from_utf8(output.stderr)?;
    assert!(
        err.starts_with("ERROR writing output") && err.lines().count() == 1,
        "one ERROR line says why: {err}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_closed_stdout_exits_2_with_one_error_line() -> TestResult {
    let scratch = Scratch::new()?;

    let output = with_closed_stdout(
        &scratch,
        &["hook", "guard", "--fail-closed"],
        &scratch.bash("rm -rf /")?,
    )?;

    assert_blocked_with_one_error_line(output)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn any_hook_passed_fail_closed_exits_2_when_its_output_cannot_be_written() -> TestResult {
    let scratch = Scratch::new()?;

    let output = with_closed_stdout(&scratch, &["hook", "bootstrap", "--fail-closed"], b"{}")?;

    assert_blocked_with_one_error_line(output)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_usage_error_exits_2() -> TestResult {
    let scratch = Scratch::new()?;

    let output = anneal_in(scratch.path())
        .args(["hook", "guard", "--no-such-flag"])
        .stdin(Stdio::null())
        .output()?;

    assert_eq!(
        output.status.code(),
        Some(2),
        "a mis-invoked guard blocks: {output:?}"
    );
    assert!(output.stdout.is_empty(), "nothing on stdout: {output:?}");
    Ok(())
}
