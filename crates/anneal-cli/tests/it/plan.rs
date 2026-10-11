//! `anneal plan approve|check` through the binary, in a scratch directory holding a spec and a
//! plan that links it, with the ledger at `ledger.sqlite3` there (`--store`).

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Output;

use serde_json::Value;
use tempfile::TempDir;

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

const PLAN: &str = "docs/plans/export.md";
const STORE: &str = "ledger.sqlite3";

const AGREED_SPEC: &str = "# Spec: export\n\n## Problem\n\nAll agreed.\n";
const OPEN_SPEC: &str =
    "# Spec: export\n\n## Open questions\n\n- Delimiter? [NEEDS CLARIFICATION: comma or tab]\n";

const CLEAN_PLAN: &str = "\
# Plan: export

- Spec: [export](../specs/export.md)

## Goal

Analysts export a report as CSV.

## Tasks

### T1: CSV writer

- Files: `src/csv.rs`
- Test: `cargo test -p report csv`
- Blocked by: none

## Review focus

- Quoting.
";

/// A scratch directory with the spec at `docs/specs/export.md` and the plan at [`PLAN`].
fn workspace(spec: &str, plan: &str) -> Result<TempDir, Box<dyn Error>> {
    let work = tempfile::tempdir()?;
    fs::create_dir_all(work.path().join("docs/specs"))?;
    fs::create_dir_all(work.path().join("docs/plans"))?;
    fs::write(work.path().join("docs/specs/export.md"), spec)?;
    fs::write(work.path().join(PLAN), plan)?;
    Ok(work)
}

/// Runs `anneal <args>` in `cwd` and parses stdout as the envelope (`Null` when empty).
fn anneal(cwd: &Path, args: &[&str]) -> Result<(Output, Value), Box<dyn Error>> {
    let output = anneal_in(cwd).args(args).output()?;
    let envelope = if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)?
    };
    Ok((output, envelope))
}

fn plan(cwd: &Path, action: &str, file: &str) -> Result<(Output, Value), Box<dyn Error>> {
    anneal(cwd, &["plan", action, file, "--store", STORE])
}

/// The `plan_approved` events in the scratch ledger.
fn approvals(cwd: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let (output, envelope) = anneal(
        cwd,
        &[
            "ledger",
            "query",
            "--type",
            "plan_approved",
            "--store",
            STORE,
        ],
    )?;
    assert_eq!(output.status.code(), Some(0), "query: {envelope}");
    Ok(envelope
        .pointer("/data/events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn keys(envelope: &Value) -> Vec<String> {
    envelope
        .get("findings")
        .and_then(Value::as_array)
        .map(|findings| {
            findings
                .iter()
                .filter_map(|finding| finding.get("key").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_approved_plan_checks_current_until_one_byte_changes() -> TestResult {
    let work = workspace(AGREED_SPEC, CLEAN_PLAN)?;

    let (approved, envelope) = plan(work.path(), "approve", PLAN)?;
    assert_eq!(approved.status.code(), Some(0), "approve: {envelope}");
    let hash = envelope
        .pointer("/data/hash")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        hash.starts_with("sha256:") && hash.len() == 71,
        "a sha256 hash: {hash}"
    );
    let events = approvals(work.path())?;
    assert_eq!(events.len(), 1, "one approval: {events:?}");
    assert_eq!(
        events
            .first()
            .and_then(|event| event.pointer("/event/plan")),
        Some(&Value::from(PLAN)),
        "the plan id is its path"
    );
    let (current, envelope) = plan(work.path(), "check", PLAN)?;
    assert_eq!(current.status.code(), Some(0), "current: {envelope}");

    fs::write(
        work.path().join(PLAN),
        CLEAN_PLAN.replacen("Quoting.", "Quoting!", 1),
    )?;
    let (stale, envelope) = plan(work.path(), "check", PLAN)?;

    assert_eq!(stale.status.code(), Some(4), "stale: {envelope}");
    assert_eq!(
        envelope.pointer("/data/status"),
        Some(&Value::from("stale")),
        "status names it"
    );
    assert_eq!(envelope.get("ok"), Some(&Value::Bool(false)), "not ok");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn approving_the_changed_plan_again_makes_it_current() -> TestResult {
    let work = workspace(AGREED_SPEC, CLEAN_PLAN)?;
    let (first, _) = plan(work.path(), "approve", PLAN)?;
    assert_eq!(first.status.code(), Some(0), "first approval");
    fs::write(work.path().join(PLAN), CLEAN_PLAN.replace("CSV.", "TSV."))?;

    let (again, envelope) = plan(work.path(), "approve", PLAN)?;
    let (check, _) = plan(work.path(), "check", PLAN)?;

    assert_eq!(again.status.code(), Some(0), "re-approve: {envelope}");
    assert_eq!(check.status.code(), Some(0), "current again");
    assert_eq!(approvals(work.path())?.len(), 2, "two approvals");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn approving_the_same_content_twice_records_one_approval() -> TestResult {
    let work = workspace(AGREED_SPEC, CLEAN_PLAN)?;
    let (first, _) = plan(work.path(), "approve", PLAN)?;
    assert_eq!(first.status.code(), Some(0), "first approval");

    let (second, envelope) = plan(work.path(), "approve", PLAN)?;

    assert_eq!(
        second.status.code(),
        Some(0),
        "already approved: {envelope}"
    );
    assert_eq!(
        envelope.pointer("/data/appended"),
        Some(&Value::Bool(false)),
        "nothing appended"
    );
    assert_eq!(approvals(work.path())?.len(), 1, "still one approval");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn approval_is_refused_while_the_spec_has_an_open_marker() -> TestResult {
    let work = workspace(OPEN_SPEC, CLEAN_PLAN)?;

    let (refused, envelope) = plan(work.path(), "approve", PLAN)?;

    assert_eq!(refused.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        keys(&envelope),
        vec!["plan/approve/needs-clarification:docs/plans/../specs/export.md".to_owned()],
        "one finding naming the spec"
    );
    assert_eq!(
        envelope.pointer("/data/needs_clarification"),
        Some(&Value::from(1)),
        "the count"
    );
    assert!(!work.path().join(STORE).exists(), "no ledger opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn approval_is_refused_when_the_plan_fails_its_lint() -> TestResult {
    let work = workspace(
        AGREED_SPEC,
        &CLEAN_PLAN.replace("- Test: `cargo test -p report csv`\n", ""),
    )?;

    let (refused, envelope) = plan(work.path(), "approve", PLAN)?;

    assert_eq!(refused.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        keys(&envelope),
        vec!["lint/plan/task-fields:docs/plans/export.md:11:Test".to_owned()],
        "the lint finding"
    );
    assert!(!work.path().join(STORE).exists(), "no ledger opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_unapproved_plan_checks_not_approved_without_creating_a_ledger() -> TestResult {
    let work = workspace(AGREED_SPEC, CLEAN_PLAN)?;

    let (check, envelope) = plan(work.path(), "check", PLAN)?;

    assert_eq!(check.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        envelope.pointer("/data/status"),
        Some(&Value::from("not_approved")),
        "status names it"
    );
    assert!(!work.path().join(STORE).exists(), "no ledger created");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_plan_id_drops_current_directory_components() -> TestResult {
    let work = workspace(AGREED_SPEC, CLEAN_PLAN)?;
    let (approved, _) = plan(work.path(), "approve", &format!("./{PLAN}"))?;
    assert_eq!(approved.status.code(), Some(0), "approved");

    let (check, envelope) = plan(work.path(), "check", "docs/./plans/export.md")?;

    assert_eq!(check.status.code(), Some(0), "the same plan: {envelope}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_plan_is_a_usage_error() -> TestResult {
    let work = workspace(AGREED_SPEC, CLEAN_PLAN)?;
    for action in ["approve", "check"] {
        let (output, _) = plan(work.path(), action, "docs/plans/none.md")?;

        assert_eq!(output.status.code(), Some(2), "{action}: usage");
        assert!(output.stdout.is_empty(), "{action}: nothing on stdout");
        assert_eq!(
            String::from_utf8(output.stderr)?,
            "ERROR plan: no such path: docs/plans/none.md\n",
            "{action}: one line"
        );
    }
    Ok(())
}
