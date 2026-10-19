//! `anneal status` through the binary, in a scratch directory outside git with the ledger at
//! `$ANNEAL_STORE` there.

use std::error::Error;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

const STORE: &str = "ledger.sqlite3";

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn without_a_ledger_status_exits_2_and_creates_none() -> TestResult {
    let work = tempfile::tempdir()?;

    let output = anneal_in(work.path())
        .args(["status", "--json"])
        .env("ANNEAL_STORE", STORE)
        .output()?;

    assert_eq!(output.status.code(), Some(2), "no ledger");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["ok"], json!(false), "ok agrees with the exit code");
    assert_eq!(
        envelope["data"]["schema"],
        json!(1),
        "the document is versioned"
    );
    assert!(output.stderr.is_empty(), "no ERROR line: it is an outcome");
    assert!(!work.path().join(STORE).exists(), "no ledger was created");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_a_ledger_status_exits_0_and_lists_its_events() -> TestResult {
    let work = tempfile::tempdir()?;
    let appended = anneal_in(work.path())
        .args(["ledger", "append", r#"{"type":"task_done","task":"T1"}"#])
        .env("ANNEAL_STORE", STORE)
        .output()?;
    assert!(appended.status.success(), "the event was appended");

    let output = anneal_in(work.path())
        .args(["status", "--json", "--fields", "ledger_tail"])
        .env("ANNEAL_STORE", STORE)
        .output()?;

    assert_eq!(output.status.code(), Some(0), "a ledger exists");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["command"], json!("status"), "the command");
    assert_eq!(
        envelope["data"]["ledger_tail"][0]["summary"],
        json!("task_done task:T1"),
        "the one event"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_unknown_section_is_a_usage_error() -> TestResult {
    let work = tempfile::tempdir()?;

    let output = anneal_in(work.path())
        .args(["status", "--fields", "phase,nope"])
        .output()?;

    assert_eq!(output.status.code(), Some(2), "a usage error");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    Ok(())
}
