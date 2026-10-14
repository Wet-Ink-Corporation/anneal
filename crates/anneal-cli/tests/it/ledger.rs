//! `anneal ledger` through the built binary: appends, queries, tails, journals, schema errors,
//! worktree sharing and concurrent writers.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;

use crate::support::{anneal_in, repo_with_linked_worktree};

type TestResult = Result<(), Box<dyn Error>>;

const TASK_DONE: &str = r#"{"type":"task_done","task":"T1"}"#;

/// Runs `anneal ledger <args>` in `cwd`.
fn ledger(cwd: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(cwd).arg("ledger").args(args).output()?)
}

/// Runs `anneal ledger --store <store> <args>` in `cwd`.
fn ledger_at(cwd: &Path, store: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(cwd)
        .arg("ledger")
        .arg("--store")
        .arg(store)
        .args(args)
        .output()?)
}

/// The JSON envelope on stdout, after checking the exit code.
fn envelope(output: &Output, code: i32) -> Result<Value, Box<dyn Error>> {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn positions(envelope: &Value) -> Vec<u64> {
    envelope
        .pointer("/data/events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter_map(|event| event.get("position").and_then(Value::as_u64))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_appended_event_is_queried_back_in_the_envelope() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join("ledger.sqlite3");

    let appended = envelope(
        &ledger_at(
            dir.path(),
            &store,
            &["append", TASK_DONE, "--session", "s1"],
        )?,
        0,
    )?;
    let queried = envelope(&ledger_at(dir.path(), &store, &["query"])?, 0)?;

    assert_eq!(appended["command"], "ledger append", "{appended}");
    assert_eq!(appended["data"]["position"], 1, "{appended}");
    let event = &queried["data"]["events"][0]["event"];
    assert_eq!(
        (
            &event["type"],
            &event["task"],
            &event["actor"],
            &event["session_id"]
        ),
        (
            &Value::from("task_done"),
            &Value::from("T1"),
            &Value::from("cli"),
            &Value::from("s1")
        ),
        "the payload and the filled-in context: {queried}"
    );
    assert!(
        event["ts"].as_str().is_some_and(|ts| ts.ends_with('Z')),
        "ts filled: {event}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_bad_event_exits_2_with_an_envelope_and_writes_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join("ledger.sqlite3");

    for bad in [
        r#"{"type":"task_started","task":"T1"}"#,
        r#"{"type":"task_done","task":"T1","extra":1}"#,
        r#"{"type":"task_done"}"#,
        "not json",
    ] {
        let output = ledger_at(dir.path(), &store, &["append", bad])?;
        let refused = envelope(&output, 2)?;

        assert_eq!(refused["ok"], false, "{bad}: {refused}");
        assert_eq!(
            refused["findings"][0]["key"], "ledger/schema",
            "{bad}: {refused}"
        );
        assert!(
            output.stderr.is_empty(),
            "{bad}: the envelope is the report"
        );
    }
    assert!(
        !store.exists(),
        "nothing was written, not even an empty ledger"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn query_filters_and_tail_keeps_the_last_events() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join("ledger.sqlite3");
    for (event, session) in [
        (TASK_DONE, "s1"),
        (r#"{"type":"task_done","task":"T2"}"#, "s2"),
        (r#"{"type":"round_started","pr":7,"round":1}"#, "s2"),
    ] {
        envelope(
            &ledger_at(dir.path(), &store, &["append", event, "--session", session])?,
            0,
        )?;
    }

    let by_type = envelope(
        &ledger_at(dir.path(), &store, &["query", "--type", "task_done"])?,
        0,
    )?;
    let by_tag = envelope(
        &ledger_at(dir.path(), &store, &["query", "--tag", "pr:7"])?,
        0,
    )?;
    let by_session = envelope(
        &ledger_at(dir.path(), &store, &["query", "--session", "s2"])?,
        0,
    )?;
    let tail = envelope(&ledger_at(dir.path(), &store, &["tail", "-n", "2"])?, 0)?;

    assert_eq!(positions(&by_type), [1, 2], "--type");
    assert_eq!(positions(&by_tag), [3], "--tag");
    assert_eq!(positions(&by_session), [2, 3], "--session");
    assert_eq!(positions(&tail), [2, 3], "tail -n 2, oldest first");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn journal_prints_every_event_as_one_json_line() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join("ledger.sqlite3");
    for event in [TASK_DONE, r#"{"type":"grill_confirmed","plan":"p.md"}"#] {
        envelope(&ledger_at(dir.path(), &store, &["append", event])?, 0)?;
    }

    let output = ledger_at(dir.path(), &store, &["journal"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let lines: Vec<Value> = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let types: Vec<&Value> = lines.iter().map(|line| &line["type"]).collect();
    assert_eq!(
        types,
        ["task_done", "grill_confirmed"],
        "one envelope per line, in order"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn the_main_checkout_and_a_linked_worktree_share_one_store() -> TestResult {
    let root = tempfile::tempdir()?;
    repo_with_linked_worktree(root.path());
    let (main, linked) = (root.path().join("main"), root.path().join("linked"));

    let appended = envelope(&ledger(&main, &["append", TASK_DONE])?, 0)?;
    let queried = envelope(&ledger(&linked, &["query"])?, 0)?;

    let store_of = |envelope: &Value| -> Result<PathBuf, Box<dyn Error>> {
        let path = envelope["data"]["store"].as_str().ok_or("no store path")?;
        Ok(std::fs::canonicalize(path)?)
    };
    assert_eq!(store_of(&appended)?, store_of(&queried)?, "one file");
    assert_eq!(
        store_of(&appended)?,
        std::fs::canonicalize(main.join(".git/anneal/ledger.sqlite3"))?,
        "inside the git common dir"
    );
    assert_eq!(queried["data"]["source"], "git_common_dir", "{queried}");
    assert_eq!(
        positions(&queried),
        [1],
        "the linked worktree sees main's event"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn twenty_parallel_processes_append_twenty_events() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join("anneal/ledger.sqlite3");

    let children = (0..20)
        .map(|n| {
            anneal_in(dir.path())
                .arg("ledger")
                .arg("--store")
                .arg(&store)
                .args([
                    "append",
                    &format!(r#"{{"type":"task_done","task":"T{n}"}}"#),
                ])
                .stdout(std::process::Stdio::null())
                .spawn()
        })
        .collect::<Result<Vec<_>, _>>()?;
    for mut child in children {
        assert!(child.wait()?.success(), "every append exits 0");
    }

    let queried = envelope(&ledger_at(dir.path(), &store, &["query"])?, 0)?;
    assert_eq!(
        positions(&queried),
        (1..=20).collect::<Vec<_>>(),
        "20 events"
    );
    Ok(())
}
