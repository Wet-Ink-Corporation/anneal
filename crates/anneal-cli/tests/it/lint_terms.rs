//! `anneal lint terms` through the binary: the envelope, its hits and the exit code.

use std::error::Error;
use std::fs;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

const GLOSSARY: &str = "**Ruling**: a scope decision.\n_Avoid_: verdict, purchase order\n";

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_hit_exits_1_with_file_line_and_term() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("CONTEXT.md"), GLOSSARY)?;
    fs::write(
        dir.path().join("spec.md"),
        "# Spec\nThe Purchase Order verdict.\n",
    )?;

    let output = anneal_in(dir.path())
        .args(["lint", "terms", "spec.md"])
        .output()?;

    assert_eq!(output.status.code(), Some(1), "a hit is an error finding");
    assert!(output.stderr.is_empty(), "findings are not errors");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["ok"], json!(false), "ok follows the exit code");
    assert_eq!(envelope["command"], json!("lint terms"), "target named");
    assert_eq!(
        envelope["data"]["hits"],
        json!([
            {"file": "spec.md", "line": 2, "term": "purchase order"},
            {"file": "spec.md", "line": 2, "term": "verdict"},
        ]),
        "one hit per term, with file and line"
    );
    assert_eq!(
        envelope["findings"][0]["key"],
        json!("lint/terms:spec.md:2:purchase order"),
        "stable key per file, line and term"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn clean_files_exit_0_and_a_named_glossary_is_read() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("terms.md"), GLOSSARY)?;
    fs::write(
        dir.path().join("spec.md"),
        "The ruling stands; verdicts differ.\n",
    )?;

    let output = anneal_in(dir.path())
        .args(["lint", "terms", "--glossary", "terms.md", "spec.md"])
        .output()?;

    assert_eq!(output.status.code(), Some(0), "no hit");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        envelope["data"],
        json!({"glossary": "terms.md", "terms": 2, "checked": 1, "skipped": [], "hits": []}),
        "counts and empty lists"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_glossary_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("spec.md"), "text\n")?;

    let output = anneal_in(dir.path())
        .args(["lint", "terms", "spec.md"])
        .output()?;

    assert_eq!(output.status.code(), Some(2), "usage error");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR lint: no such path: CONTEXT.md\n",
        "one ERROR line naming the glossary"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_file_named_with_and_without_dot_slash_is_scanned_once() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("CONTEXT.md"), GLOSSARY)?;
    fs::write(dir.path().join("a.md"), "a verdict\n")?;

    let output = anneal_in(dir.path())
        .args(["lint", "terms", "a.md", "./a.md"])
        .output()?;

    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        envelope["data"]["hits"],
        json!([{"file": "a.md", "line": 1, "term": "verdict"}]),
        "one file, named without its `./`"
    );
    assert_eq!(envelope["data"]["checked"], json!(1), "scanned once");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn no_files_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("CONTEXT.md"), GLOSSARY)?;

    let output = anneal_in(dir.path()).args(["lint", "terms"]).output()?;

    assert_eq!(output.status.code(), Some(2), "usage error");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR the following required arguments were not provided:\n",
        "one ERROR line, clap's first"
    );
    Ok(())
}
