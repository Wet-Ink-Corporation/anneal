//! `anneal task done` through the built binary: `task_done` is recorded only after a green test
//! command, a red one records only its `test_run`, and one that cannot start records nothing.
//!
//! The test commands are `git` subcommands, which exist on every CI platform: `git --version`
//! exits 0 and `git definitely-not-a-command` exits 1.

use std::error::Error;
use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// Runs `anneal task done <args>` in `cwd` against `<cwd>/ledger.sqlite3`.
fn task_done(cwd: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(cwd)
        .args(["task", "done", "--store", "ledger.sqlite3"])
        .args(args)
        .output()?)
}

/// The events in `<cwd>/ledger.sqlite3`, oldest first.
fn events(cwd: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let output = anneal_in(cwd)
        .args(["ledger", "--store", "ledger.sqlite3", "query"])
        .output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let listing: Value = serde_json::from_slice(&output.stdout)?;
    let events = listing
        .pointer("/data/events")
        .and_then(Value::as_array)
        .ok_or("events")?;
    // The caller owns the events it inspects.
    Ok(events
        .iter()
        .filter_map(|recorded| recorded.get("event").cloned())
        .collect())
}

/// `(type, task, outcome, exit)` of each event, the fields every assertion here compares.
fn summary(events: &[Value]) -> Vec<(Value, Value, Value, Value)> {
    let field = |event: &Value, key: &str| event.get(key).cloned().unwrap_or(Value::Null);
    events
        .iter()
        .map(|event| {
            (
                field(event, "type"),
                field(event, "task"),
                field(event, "outcome"),
                field(event, "exit"),
            )
        })
        .collect()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_green_command_records_its_test_run_then_task_done() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = task_done(
        dir.path(),
        &["T1", "--session", "s1", "--", "git", "--version"],
    )?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let recorded = events(dir.path())?;
    assert_eq!(
        summary(&recorded),
        vec![
            (json!("test_run"), json!("T1"), json!("green"), json!(0)),
            (json!("task_done"), json!("T1"), Value::Null, Value::Null),
        ],
        "a green test_run, then task_done"
    );
    let run = recorded.first().ok_or("a test_run")?;
    assert_eq!(run["command"], "git --version", "the command as run");
    assert!(run["duration_ms"].is_u64(), "a duration in ms: {run}");
    assert_eq!(run["timeout_secs"], Value::Null, "it did not time out");
    assert_eq!(run["session_id"], "s1", "the session is recorded");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        (
            &envelope["ok"],
            &envelope["command"],
            &envelope["data"]["task"]
        ),
        (&json!(true), &json!("task done"), &json!("T1")),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_failing_command_records_a_red_test_run_and_no_task_done() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = task_done(dir.path(), &["T1", "--", "git", "definitely-not-a-command"])?;

    assert_eq!(output.status.code(), Some(1), "test red: {output:?}");
    assert_eq!(
        summary(&events(dir.path())?),
        vec![(json!("test_run"), json!("T1"), json!("red"), json!(1))],
        "only the failed test_run"
    );
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["ok"], json!(false), "{envelope}");
    assert_eq!(
        envelope["findings"][0]["key"],
        json!("task/red:T1"),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_command_that_cannot_start_exits_2_and_opens_no_ledger() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = task_done(dir.path(), &["T1", "--", "anneal-no-such-program-22"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope for an error");
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stderr.lines().count(), 1, "one line: {stderr}");
    assert!(
        stderr.starts_with("ERROR task: could not start anneal-no-such-program-22"),
        "{stderr}"
    );
    assert!(
        !dir.path().join("ledger.sqlite3").exists(),
        "no ledger was opened"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_test_commands_output_goes_to_stderr_and_stdout_stays_the_envelope() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = task_done(dir.path(), &["T1", "--", "git", "--version"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("git version"),
        "passed through: {stderr}"
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.lines().count(), 1, "one line: {stdout}");
    let envelope: Value = serde_json::from_str(&stdout)?;
    assert_eq!(envelope["command"], "task done", "{envelope}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_command_or_zero_timeout_is_a_usage_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    for args in [
        &["T1"][..],
        &["T1", "--"],
        &["T1", "--timeout", "0", "--", "git", "--version"],
    ] {
        let output = task_done(dir.path(), args)?;

        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: nothing on stdout");
        assert_eq!(
            String::from_utf8(output.stderr)?.lines().count(),
            1,
            "{args:?}: one ERROR line"
        );
    }
    assert!(
        !dir.path().join("ledger.sqlite3").exists(),
        "no ledger was opened"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_task_id_the_ledger_cannot_tag_is_a_usage_error_and_runs_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    let long_id = "T".repeat(300);

    let output = task_done(dir.path(), &[&long_id, "--", "git", "--version"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        !stderr.contains("git version"),
        "the command did not run: {stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "one ERROR line: {stderr}");
    assert!(
        !dir.path().join("ledger.sqlite3").exists(),
        "no ledger was opened"
    );
    Ok(())
}
