//! The define chain replayed on the dry run recorded in `evals/fixtures/dry-run/`: a design
//! tree with nothing open is confirmed, the spec and plan lint clean, the plan is approved and
//! current, the tickets check clean with a known frontier, and the ledger events equal the
//! recorded `events.jsonl`. Each test runs the built binary on a temporary copy of the
//! fixture, with the ledger at `ledger.sqlite3` there (`--store`).

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, copy_dir, sha256_hex};

type TestResult = Result<(), Box<dyn Error>>;

const STORE: &str = "ledger.sqlite3";
const TREE: &str = "design-tree.json";
const SPEC: &str = "spec.md";
const PLAN: &str = "plan.md";
/// The human's confirmation, as recorded in `events.jsonl`.
const QUOTE: &str = "Yes, that's it: only notes list, compact JSON, go ahead.";
const SESSION: &str = "dry-run";
/// Envelope fields that differ from run to run, left out of `events.jsonl`.
const VOLATILE: [&str; 4] = ["ts", "worktree", "branch", "head"];

fn fixture() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "../../evals/fixtures/dry-run"]
        .iter()
        .collect()
}

/// A temporary copy of the fixture, so the ledger and any edit stay out of the repository.
fn workspace() -> Result<TempDir, Box<dyn Error>> {
    let work = tempfile::tempdir()?;
    copy_dir(&fixture(), work.path())?;
    Ok(work)
}

/// Runs `anneal <args>` in `cwd`, checks it exits `code` with nothing on stderr, and returns
/// the envelope.
fn anneal(cwd: &Path, args: &[&str], code: i32) -> Result<Value, Box<dyn Error>> {
    let output = anneal_in(cwd).args(args).output()?;
    assert_eq!(
        output.status.code(),
        Some(code),
        "anneal {args:?}: {output:?}"
    );
    assert!(
        output.stderr.is_empty(),
        "anneal {args:?}: stderr: {output:?}"
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn confirm(cwd: &Path) -> Result<Value, Box<dyn Error>> {
    anneal(
        cwd,
        &[
            "grill",
            "confirm",
            "--tree",
            TREE,
            "--quote",
            QUOTE,
            "--session",
            SESSION,
            "--store",
            STORE,
        ],
        0,
    )
}

fn approve(cwd: &Path) -> Result<Value, Box<dyn Error>> {
    anneal(
        cwd,
        &[
            "plan",
            "approve",
            PLAN,
            "--session",
            SESSION,
            "--store",
            STORE,
        ],
        0,
    )
}

/// The bytes of the fixture file `name`, with an error that names it.
fn read_fixture(name: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let path = fixture().join(name);
    fs::read(&path).map_err(|error| format!("{}: {error}", path.display()).into())
}

/// The lines of `events.jsonl`, each a `{position, event}` as `ledger query` lists it.
fn recorded_events() -> Result<Vec<Value>, Box<dyn Error>> {
    Ok(String::from_utf8(read_fixture("events.jsonl")?)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?)
}

/// `{position, event}` with the [`VOLATILE`] keys removed from the event.
fn normalised(mut record: Value) -> Value {
    if let Some(event) = record.get_mut("event").and_then(Value::as_object_mut) {
        for key in VOLATILE {
            event.remove(key);
        }
    }
    record
}

fn ids(tickets: &Value) -> Vec<&str> {
    tickets
        .as_array()
        .map(|tickets| {
            tickets
                .iter()
                .filter_map(|ticket| ticket["id"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_design_tree_has_no_open_question_and_its_confirmation_is_appended() -> TestResult {
    let work = workspace()?;

    let frontier = anneal(work.path(), &["grill", "frontier", "--tree", TREE], 0)?;
    let confirmed = confirm(work.path())?;

    let data = &frontier["data"];
    assert_eq!(
        (
            &data["frontier"],
            &data["open"],
            &data["answered"],
            &data["deferred"]
        ),
        (&json!([]), &json!(0), &json!(7), &json!(1)),
        "every node answered or deferred"
    );
    assert_eq!(
        data["confidence"]["restate"],
        json!(true),
        "the restate gate was reached"
    );
    assert_eq!(
        confirmed["data"]["position"],
        json!(1),
        "the first event in a fresh ledger"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_spec_lints_clean_against_the_constitution_with_no_open_marker() -> TestResult {
    let work = workspace()?;

    let linted = anneal(work.path(), &["lint", "spec", SPEC], 0)?;

    assert_eq!(
        linted["data"],
        json!({
            "file": SPEC,
            "glossary": "CONTEXT.md",
            "constitution": {"file": "constitution.md", "version": "1", "gates": 3},
            "criteria": 5,
            "needs_clarification": 0,
            "findings": [],
        }),
        "the glossary and the constitution were read, five EARS criteria, no marker"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_plan_lints_clean_and_its_approval_checks_current() -> TestResult {
    let work = workspace()?;

    let linted = anneal(work.path(), &["lint", "plan", PLAN], 0)?;
    let approved = approve(work.path())?;
    let checked = anneal(work.path(), &["plan", "check", PLAN, "--store", STORE], 0)?;

    assert_eq!(
        (&linted["data"]["spec"], &linted["data"]["tasks"]),
        (&json!(SPEC), &json!(4)),
        "the plan links the spec and has four tasks"
    );
    assert_eq!(
        approved["data"]["appended"],
        json!(true),
        "one approval appended"
    );
    assert_eq!(
        checked["data"]["status"],
        json!("current"),
        "the approval is of the current bytes"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_tickets_check_clean_and_the_frontier_is_t1_and_t4() -> TestResult {
    let work = workspace()?;

    let checked = anneal(work.path(), &["dag", "check", "--dir", "tickets"], 0)?;
    let frontier = anneal(work.path(), &["dag", "frontier", "--dir", "tickets"], 0)?;

    assert_eq!(
        checked["data"],
        json!({"tracker": "markdown", "tickets": 4, "done": 0, "edges": 2, "dangling": [], "cycle": []}),
        "four open tickets, T2 and T3 each blocked by T1"
    );
    assert_eq!(
        ids(&frontier["data"]["tickets"]),
        ["T1", "T4"],
        "the tickets blocked by nothing"
    );
    Ok(())
}

#[test]
fn the_recorded_hashes_are_those_of_the_checked_in_tree_and_plan() -> TestResult {
    let recorded = recorded_events()?;

    let hashes: Vec<&Value> = recorded
        .iter()
        .map(|record| &record["event"]["hash"])
        .collect();

    assert_eq!(
        hashes,
        [
            &json!(sha256_hex(&read_fixture(TREE)?)?),
            &json!(sha256_hex(&read_fixture(PLAN)?)?),
        ],
        "a fixture changed without re-recording events.jsonl, or was checked out with CRLF"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_replayed_ledger_events_equal_the_recorded_ones() -> TestResult {
    let work = workspace()?;
    let recorded = recorded_events()?;

    confirm(work.path())?;
    approve(work.path())?;
    let mut queried = anneal(work.path(), &["ledger", "query", "--store", STORE], 0)?;

    let Some(Value::Array(events)) = queried.pointer_mut("/data/events").map(Value::take) else {
        return Err(format!("no events array: {queried}").into());
    };
    let replayed: Vec<Value> = events.into_iter().map(normalised).collect();
    assert_eq!(
        replayed, recorded,
        "events.jsonl is the ledger of this chain"
    );
    Ok(())
}
