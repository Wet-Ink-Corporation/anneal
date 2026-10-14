//! `anneal round start|status` through the built binary. Each call is its own process, so the
//! count the next call sees can only come from the ledger: it survives a compaction, which
//! leaves the orchestrator nothing but the files and the ledger.

use std::error::Error;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A scratch repository with no ledger yet.
fn repo() -> TestResult<TempDir> {
    let dir = tempfile::tempdir()?;
    git(dir.path(), &["init", "-q"]);
    Ok(dir)
}

/// Runs `anneal round <args>` in `dir`, expecting exit `code`, nothing on stderr and an
/// envelope.
fn round(dir: &Path, args: &[&str], code: i32) -> TestResult<Value> {
    let output = anneal_in(dir).arg("round").args(args).output()?;
    assert_eq!(
        output.status.code(),
        Some(code),
        "round {args:?}: {output:?}"
    );
    assert!(output.stderr.is_empty(), "round {args:?}: {output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn ledger(dir: &Path) -> std::path::PathBuf {
    dir.join(".git").join("anneal").join("ledger.sqlite3")
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn status_before_any_round_says_the_same_agent_starts_and_creates_no_ledger() -> TestResult {
    let dir = repo()?;

    let envelope = round(dir.path(), &["status", "--ticket", "36"], 0)?;

    assert_eq!(
        (&envelope["data"]["rounds"], &envelope["data"]["next"]),
        (&json!(0), &json!("same-agent")),
        "{envelope}"
    );
    assert!(
        !ledger(dir.path()).exists(),
        "status never creates a ledger"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn the_round_count_comes_from_the_ledger_across_processes_and_parks_after_five() -> TestResult {
    let dir = repo()?;
    round(dir.path(), &["start", "--ticket", "other"], 0)?;

    let mut plans = Vec::new();
    for _ in 0..5 {
        let mut started = round(dir.path(), &["start", "--ticket", "36"], 0)?;
        let data = &mut started["data"];
        plans.push((data["started"].take(), data["next"].take()));
    }
    let status = round(dir.path(), &["status", "--ticket", "36"], 3)?;
    let refused = round(dir.path(), &["start", "--ticket", "36"], 3)?;

    assert_eq!(
        plans,
        [
            (json!(1), json!("same-agent")),
            (json!(2), json!("same-agent")),
            (json!(3), json!("same-agent")),
            (json!(4), json!("fresh-agent")),
            (json!(5), json!("fresh-agent")),
        ],
        "another ticket's round does not count"
    );
    assert_eq!(
        (&status["data"]["rounds"], &status["data"]["next"]),
        (&json!(5), &json!("park")),
        "{status}"
    );
    assert_eq!(
        (&refused["data"]["rounds"], &refused["data"]["started"]),
        (&json!(5), &Value::Null),
        "a sixth start records nothing: {refused}"
    );
    let events = anneal_in(dir.path())
        .args(["ledger", "query", "--type", "round_started"])
        .output()?;
    let events: Value = serde_json::from_slice(&events.stdout)?;
    let ticket_rounds: Vec<&Value> = events["data"]["events"]
        .as_array()
        .ok_or("events")?
        .iter()
        .filter(|recorded| recorded["event"]["ticket"] == json!("36"))
        .map(|recorded| &recorded["event"]["round"])
        .collect();
    assert_eq!(
        ticket_rounds,
        [&json!(1), &json!(2), &json!(3), &json!(4), &json!(5)],
        "five rounds recorded, one per start"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_ticket_that_is_not_a_plain_name_is_a_usage_error_that_records_nothing() -> TestResult {
    let dir = repo()?;

    let output = anneal_in(dir.path())
        .args(["round", "start", "--ticket", "../36"])
        .output()?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope for a usage error");
    assert!(!ledger(dir.path()).exists(), "nothing recorded");
    Ok(())
}
