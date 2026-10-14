//! `anneal lint plan` through the binary, in a scratch directory with a spec and a plan, and
//! the agreement of `plugin/playbooks/schemas/plan.schema.json` with its `data.plan`.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

const PLAN: &str = "\
# Plan: export

Spec: `../specs/export.md`

## Goal

Analysts export a report as CSV.

## Tasks

### T1: CSV writer

- Files: `src/csv.rs`, `tests/csv.rs`
- Test: `cargo test -p report csv`
- Blocked by: none

### T2: Export command

- Files: `src/export.rs`
- Test: `cargo test -p report export`
- Blocked by: T1

## Review focus

- Quoting.
";

/// A scratch directory with a spec at `docs/specs/export.md` (when `with_spec`) and [`PLAN`]
/// at `docs/plans/export.md`.
fn workspace(with_spec: bool) -> Result<TempDir, Box<dyn Error>> {
    let work = tempfile::tempdir()?;
    fs::create_dir_all(work.path().join("docs/specs"))?;
    fs::create_dir_all(work.path().join("docs/plans"))?;
    if with_spec {
        fs::write(work.path().join("docs/specs/export.md"), "# Spec: export\n")?;
    }
    fs::write(work.path().join("docs/plans/export.md"), PLAN)?;
    Ok(work)
}

fn lint_plan(cwd: &Path, file: &str) -> Result<(Output, Value), Box<dyn Error>> {
    let output = anneal_in(cwd).args(["lint", "plan", file]).output()?;
    let envelope = if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)?
    };
    Ok((output, envelope))
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_clean_plan_with_its_spec_exits_0_with_the_plan_as_read() -> TestResult {
    let work = workspace(true)?;

    let (output, envelope) = lint_plan(work.path(), "docs/plans/export.md")?;

    assert_eq!(output.status.code(), Some(0), "clean: {envelope}");
    assert_eq!(
        envelope.get("data"),
        Some(&json!({
            "file": "docs/plans/export.md",
            "spec": "docs/plans/../specs/export.md",
            "tasks": 2,
            "plan": {
                "spec": "../specs/export.md",
                "tasks": [
                    {"id": "T1", "title": "CSV writer", "line": 11,
                     "files": ["src/csv.rs", "tests/csv.rs"],
                     "test": "cargo test -p report csv", "blocked_by": []},
                    {"id": "T2", "title": "Export command", "line": 17,
                     "files": ["src/export.rs"],
                     "test": "cargo test -p report export", "blocked_by": ["T1"]}
                ]
            },
            "findings": []
        })),
        "data"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_plan_near_the_read_cap_full_of_repeats_reports_a_bounded_envelope() -> TestResult {
    let work = workspace(true)?;
    let repeat = "### T1: again\n\n";
    let count = (1024 * 1024 - PLAN.len()) / repeat.len();
    let repeats = repeat.repeat(count);
    let plan = PLAN.replace("## Review focus", &format!("{repeats}## Review focus"));
    fs::write(work.path().join("docs/plans/big.md"), plan)?;

    let (output, envelope) = lint_plan(work.path(), "docs/plans/big.md")?;

    assert_eq!(output.status.code(), Some(1), "findings");
    let listed = |pointer: &str| {
        envelope
            .pointer(pointer)
            .and_then(Value::as_array)
            .map(Vec::len)
    };
    assert_eq!(listed("/findings"), Some(1_000), "capped");
    assert_eq!(listed("/data/findings"), Some(1_000), "capped in data too");
    assert_eq!(
        envelope.pointer("/data/omitted"),
        Some(&Value::from(count - 1_000)),
        "the rest counted"
    );
    assert!(
        output.stdout.len() < 2 * 1024 * 1024,
        "the envelope stays small: {} bytes",
        output.stdout.len()
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_plan_whose_spec_is_missing_exits_1_with_a_spec_finding() -> TestResult {
    let work = workspace(false)?;

    let (output, envelope) = lint_plan(work.path(), "docs/plans/export.md")?;

    assert_eq!(output.status.code(), Some(1), "finding: {envelope}");
    assert_eq!(
        envelope.pointer("/findings/0/key"),
        Some(&Value::from("lint/plan/spec:docs/plans/export.md:3")),
        "at the Spec line"
    );
    assert_eq!(envelope.pointer("/findings/1"), None, "only that finding");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_plan_is_a_usage_error() -> TestResult {
    let work = workspace(true)?;

    let (output, _) = lint_plan(work.path(), "docs/plans/none.md")?;

    assert_eq!(output.status.code(), Some(2), "usage");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR lint: no such path: docs/plans/none.md\n",
        "one line"
    );
    Ok(())
}

fn schema() -> Result<Value, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugin/playbooks/schemas/plan.schema.json");
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn strings(schema: &Value, pointer: &str) -> BTreeSet<String> {
    schema
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn keys(object: Option<&Value>) -> BTreeSet<String> {
    object
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_schema_fields_agree_with_the_plan_lint_reads() -> TestResult {
    let work = workspace(true)?;
    let (_, envelope) = lint_plan(work.path(), "docs/plans/export.md")?;
    let schema = schema()?;

    for (sample, at) in [
        (envelope.pointer("/data/plan"), ""),
        (envelope.pointer("/data/plan/tasks/0"), "/$defs/task"),
    ] {
        let fields = keys(sample);
        assert!(!fields.is_empty(), "{at}: the sample has fields");
        assert_eq!(
            strings(&schema, &format!("{at}/required")),
            fields,
            "{at} required"
        );
        assert_eq!(
            keys(schema.pointer(&format!("{at}/properties"))),
            fields,
            "{at} properties"
        );
        assert_eq!(
            schema.pointer(&format!("{at}/additionalProperties")),
            Some(&Value::Bool(false)),
            "{at} is closed"
        );
    }
    Ok(())
}
