//! `anneal lint spec` through the binary, against the fixtures in `tests/fixtures/lint-spec/`:
//! `pass.md` is clean, and each `fail-<rule>.md` trips only its rule. The fixture directory
//! holds the default `CONTEXT.md` and `constitution.md`.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_with_home;

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lint-spec")
}

fn lint_spec(cwd: &Path, args: &[&str]) -> Result<(Output, Value), Box<dyn Error>> {
    let home = tempfile::tempdir()?;
    let output = anneal_with_home(cwd, home.path())
        .args(["lint", "spec"])
        .args(args)
        .output()?;
    let envelope = if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)?
    };
    Ok((output, envelope))
}

/// Each finding of `envelope` as `(rule, line)`.
fn rules_and_lines(envelope: &Value) -> Vec<(String, u64)> {
    let field = |finding: &Value, key: &str| finding.get(key).cloned().unwrap_or(Value::Null);
    envelope
        .get("data")
        .and_then(|data| data.get("findings"))
        .and_then(Value::as_array)
        .map(|findings| {
            findings
                .iter()
                .map(|finding| {
                    (
                        field(finding, "rule").as_str().unwrap_or("?").to_owned(),
                        field(finding, "line").as_u64().unwrap_or(u64::MAX),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn expect(rule: &str, lines: &[u64]) -> Vec<(String, u64)> {
    lines.iter().map(|line| (rule.to_owned(), *line)).collect()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_clean_spec_exits_0_with_counts_and_the_defaults_it_read() -> TestResult {
    let (output, envelope) = lint_spec(&fixtures(), &["pass.md"])?;

    assert_eq!(output.status.code(), Some(0), "clean: {envelope}");
    assert_eq!(envelope["command"], json!("lint spec"), "target named");
    assert_eq!(
        envelope["data"],
        json!({
            "file": "pass.md",
            "glossary": "CONTEXT.md",
            "constitution": {"file": "constitution.md", "version": "3", "gates": 3},
            "criteria": 8,
            "needs_clarification": 1,
            "findings": [],
        }),
        "every EARS form passes; markers in code are not counted"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn missing_and_repeated_sections_are_findings() -> TestResult {
    let (output, envelope) = lint_spec(&fixtures(), &["fail-sections.md"])?;

    assert_eq!(output.status.code(), Some(1), "findings exit 1");
    assert_eq!(envelope["ok"], json!(false), "ok follows the exit code");
    assert_eq!(
        rules_and_lines(&envelope),
        expect("sections", &[0, 0, 0, 23]),
        "Open questions, Requirements, Users missing; Problem repeated"
    );
    assert_eq!(
        envelope["findings"][0],
        json!({
            "key": "lint/spec/sections:fail-sections.md:0:Open questions",
            "message": "fail-sections.md:0: error [sections] no `## Open questions` section \
                        fix: add a `## Open questions` section (see `playbooks/spec.template.md`).",
        }),
        "one envelope line with key, location, rule and fix"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn every_criterion_that_is_not_ears_is_a_finding_with_a_template() -> TestResult {
    let (output, envelope) = lint_spec(&fixtures(), &["fail-ears.md"])?;

    assert_eq!(output.status.code(), Some(1), "findings exit 1");
    assert_eq!(
        rules_and_lines(&envelope),
        expect("ears", &(18..=30).collect::<Vec<u64>>()),
        "line 17 is EARS; 18 to 30 each break the grammar once"
    );
    let fixes: Vec<&str> = envelope["data"]["findings"]
        .as_array()
        .map(|all| all.iter().filter_map(|f| f["fix"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(
        fixes.first().copied(),
        Some("write it as `WHEN <trigger>, THE <system> SHALL <response>.`"),
        "the lowercase `When` gets the event-driven template"
    );
    assert_eq!(
        fixes.get(2).copied(),
        Some("write it as `IF <condition>, THEN THE <system> SHALL <response>.`"),
        "IF without THEN gets the unwanted-behaviour template"
    );
    assert_eq!(
        envelope["data"]["criteria"],
        json!(14),
        "every item counted"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_out_of_scope_with_only_a_comment_is_a_finding() -> TestResult {
    let (output, envelope) = lint_spec(&fixtures(), &["fail-out-of-scope.md"])?;

    assert_eq!(output.status.code(), Some(1), "findings exit 1");
    assert_eq!(
        rules_and_lines(&envelope),
        expect("out-of-scope", &[19]),
        "at the heading"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_fourth_marker_is_a_finding_and_the_count_is_reported() -> TestResult {
    let (output, envelope) = lint_spec(&fixtures(), &["fail-needs-clarification.md"])?;

    assert_eq!(output.status.code(), Some(1), "findings exit 1");
    assert_eq!(
        envelope["data"]["needs_clarification"],
        json!(4),
        "the comment's marker is not counted"
    );
    assert_eq!(
        rules_and_lines(&envelope),
        expect("needs-clarification", &[25]),
        "at the fourth marker"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_stale_or_unjustified_constitution_check_is_a_finding_per_problem() -> TestResult {
    let (output, envelope) = lint_spec(&fixtures(), &["fail-constitution.md"])?;

    assert_eq!(output.status.code(), Some(1), "findings exit 1");
    assert_eq!(
        rules_and_lines(&envelope),
        expect("constitution", &[27, 29, 31, 33, 34, 38, 39]),
        "G3 unchecked, version 2 not 3, G1 violated without a row, G9 unknown, G1 repeated, \
         G2's justification empty, a row for G3 which is not violated"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn avoided_terms_are_findings() -> TestResult {
    let (output, envelope) = lint_spec(&fixtures(), &["fail-avoided-term.md"])?;

    assert_eq!(output.status.code(), Some(1), "findings exit 1");
    assert_eq!(
        rules_and_lines(&envelope),
        expect("avoided-term", &[5, 5, 13]),
        "`data dump` and `dump` on line 5, `dump` on line 13"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn absent_default_glossary_and_constitution_are_null() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::copy(fixtures().join("pass.md"), dir.path().join("spec.md"))?;

    let (output, envelope) = lint_spec(dir.path(), &["spec.md"])?;

    assert_eq!(output.status.code(), Some(0), "clean: {envelope}");
    assert_eq!(envelope["data"]["glossary"], Value::Null, "no glossary");
    assert_eq!(
        envelope["data"]["constitution"],
        Value::Null,
        "no constitution"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_named_glossary_or_constitution_that_is_missing_is_a_usage_error() -> TestResult {
    for flag in ["--glossary", "--constitution"] {
        let (output, _) = lint_spec(&fixtures(), &["pass.md", flag, "nope.md"])?;

        assert_eq!(output.status.code(), Some(2), "{flag}: usage error");
        assert!(output.stdout.is_empty(), "{flag}: nothing on stdout");
        assert_eq!(
            String::from_utf8(output.stderr)?,
            "ERROR lint: no such path: nope.md\n",
            "{flag}: one ERROR line"
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_spec_is_a_usage_error() -> TestResult {
    let (output, _) = lint_spec(&fixtures(), &["nope.md"])?;

    assert_eq!(output.status.code(), Some(2), "usage error");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_spec_over_the_read_cap_is_a_usage_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("spec.md"), vec![b'a'; 1024 * 1024 + 1])?;

    let (output, _) = lint_spec(dir.path(), &["spec.md"])?;

    assert_eq!(output.status.code(), Some(2), "too large to check");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR lint: spec.md is 1048577 bytes, over the 1048576-byte read cap\n",
        "one ERROR line with the size"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_shipped_spec_template_lints_clean() -> TestResult {
    let dir = tempfile::tempdir()?;
    let template =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/playbooks/spec.template.md");
    fs::copy(template, dir.path().join("spec.template.md"))?;

    let (output, envelope) = lint_spec(dir.path(), &["spec.template.md"])?;

    assert_eq!(output.status.code(), Some(0), "template clean: {envelope}");
    assert_eq!(
        envelope["data"]["needs_clarification"],
        json!(0),
        "no marker"
    );
    Ok(())
}
