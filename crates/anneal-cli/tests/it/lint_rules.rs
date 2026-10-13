//! `anneal lint rules` through the binary, over the fixtures in `tests/fixtures/lint-rules/`,
//! resolved against this repository (its hooks metadata and its test files).

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("lint-rules")
        .join(name)
}

/// `anneal lint rules <fixture>` from the repository root.
fn lint_rules(name: &str) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(&repository())
        .args(["lint", "rules"])
        .arg(fixture(name))
        .output()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_rule_with_no_enforcer_is_an_error() -> TestResult {
    let output = lint_rules("fail.md")?;

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(envelope["command"], json!("lint rules"));
    let details = envelope["data"]["details"].as_array().expect("details");
    assert_eq!(
        details.len(),
        1,
        "only the rule with no enforcer: {details:#?}"
    );
    let detail = details.first().expect("one finding");
    assert_eq!(
        (
            &detail["checker"],
            &detail["rule"],
            &detail["severity"],
            &detail["line"]
        ),
        (&json!("enforcer"), &json!(10), &json!("error"), &json!(6)),
        "{detail}"
    );
    assert_eq!(detail["message"], json!("this rule names no enforcer."));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn rules_naming_a_real_hook_command_and_test_are_clean() -> TestResult {
    let output = lint_rules("pass.md")?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &envelope(&output)?["data"];
    assert_eq!(
        (&data["checked"], &data["errors"], &data["details"]),
        (&json!(1), &json!(0), &json!([]))
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_rule_document_is_a_usage_error() -> TestResult {
    let output = lint_rules("no-such-file.md")?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    let stderr = std::str::from_utf8(&output.stderr)?;
    assert!(stderr.starts_with("ERROR "), "{stderr}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_or_file_root_is_a_usage_error() -> TestResult {
    let scratch = tempfile::tempdir()?;
    let file = scratch.path().join("a-file");
    std::fs::write(&file, "")?;

    for root in [scratch.path().join("missing"), file] {
        let output = anneal_in(&repository())
            .args(["lint", "rules"])
            .arg(fixture("pass.md"))
            .arg("--root")
            .arg(&root)
            .output()?;

        assert_eq!(
            output.status.code(),
            Some(2),
            "{}: {output:?}",
            root.display()
        );
        assert!(output.stdout.is_empty(), "nothing on stdout");
        let stderr = std::str::from_utf8(&output.stderr)?;
        assert!(stderr.starts_with("ERROR "), "{stderr}");
    }
    Ok(())
}
