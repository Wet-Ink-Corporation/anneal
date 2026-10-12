use std::fs;
use std::path::Path;

use serde_json::json;

use super::{Coverage, SuiteKind, read};
use crate::eval::uplift::case::Fault;
use crate::lint::skills::LintError;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn case_json() -> String {
    json!({
        "behaviour": "outcome",
        "seed": "a real failure",
        "prompt": "Plan the export feature.",
        "graders": [{"type": "regex", "name": "tasks", "pattern": "task"}]
    })
    .to_string()
}

fn write(root: &Path, relative: &str, text: &str) -> std::io::Result<()> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

fn skills() -> Vec<String> {
    ["build", "plan"].map(str::to_owned).to_vec()
}

#[test]
fn cases_are_read_by_skill_suite_and_name_with_the_coverage_of_each_skill() -> TestResult {
    let dir = tempfile::tempdir()?;
    write(dir.path(), "plan/regression/b-two.json", &case_json())?;
    write(dir.path(), "plan/regression/a-one.json", &case_json())?;
    write(dir.path(), "plan/capability/c.json", &case_json())?;
    write(dir.path(), "plan/README.md", "notes")?;
    write(dir.path(), "README.md", "notes")?;

    let suites = read(dir.path(), &skills())?;

    let names: Vec<String> = suites.cases.iter().map(super::Case::name).collect();
    assert_eq!(
        names,
        [
            "plan--capability--c",
            "plan--regression--a-one",
            "plan--regression--b-two"
        ]
    );
    assert_eq!(
        (
            suites.cases.first().map(|case| case.file.as_str()),
            suites.problems.len()
        ),
        (Some("plan/capability/c.json"), 0)
    );
    assert_eq!(
        suites.coverage,
        [Coverage {
            skill: "plan".to_owned(),
            regression: 2,
            capability: 1
        }]
    );
    Ok(())
}

#[test]
fn a_bad_layout_or_case_is_a_problem_naming_its_file() -> TestResult {
    let dir = tempfile::tempdir()?;
    write(dir.path(), "plan/regression/ok.json", &case_json())?;
    write(dir.path(), "plan/regression/Bad_Name.json", &case_json())?;
    write(dir.path(), "plan/regression/broken.json", "{")?;
    write(dir.path(), "plan/smoke/x.json", &case_json())?;
    write(dir.path(), "plan/capability/notes.txt", "no cases")?;
    write(dir.path(), "nosuch/regression/x.json", &case_json())?;

    let suites = read(dir.path(), &skills())?;

    let problems: Vec<(&str, &str)> = suites
        .problems
        .iter()
        .map(|p| (p.file.as_str(), p.reason.as_str()))
        .collect();
    assert_eq!(
        problems,
        [
            ("nosuch", "no skill `nosuch` under the skills directory"),
            ("plan/capability", "the suite has no case"),
            (
                "plan/regression/Bad_Name.json",
                "a case name is 1 to 64 of a-z, 0-9 and -, starting with a letter or digit"
            ),
            (
                "plan/regression/broken.json",
                "not a case: EOF while parsing an object at line 1 column 1"
            ),
            ("plan/smoke", "a suite is `regression` or `capability`"),
        ]
    );
    assert_eq!(suites.cases.len(), 1, "the valid case is still read");
    Ok(())
}

#[test]
fn a_case_fault_is_reported_through_its_message() -> TestResult {
    let dir = tempfile::tempdir()?;
    let text = case_json().replace("Plan the export feature.", " ");
    write(dir.path(), "plan/regression/blank.json", &text)?;

    let suites = read(dir.path(), &skills())?;

    assert_eq!(
        suites.problems.first().map(|p| p.reason.clone()),
        Some(Fault::Blank("prompt").to_string())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn a_symlinked_case_or_suite_is_not_followed() -> TestResult {
    let dir = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    write(elsewhere.path(), "x.json", &case_json())?;
    fs::create_dir_all(dir.path().join("plan/regression"))?;
    std::os::unix::fs::symlink(
        elsewhere.path().join("x.json"),
        dir.path().join("plan/regression/x.json"),
    )?;
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("plan/capability"))?;
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("build"))?;

    let suites = read(dir.path(), &skills())?;

    let problems: Vec<(&str, &str)> = suites
        .problems
        .iter()
        .map(|p| (p.file.as_str(), p.reason.as_str()))
        .collect();
    assert_eq!(
        problems,
        [
            (
                "build",
                "a symlink; suites and cases are read only as plain directories and files"
            ),
            (
                "plan/capability",
                "a symlink; suites and cases are read only as plain directories and files"
            ),
            ("plan/regression", "the suite has no case"),
            (
                "plan/regression/x.json",
                "a symlink; suites and cases are read only as plain directories and files"
            ),
        ]
    );
    Ok(())
}

#[test]
fn a_missing_suites_directory_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");

    let missing = read(&dir.path().join("none"), &skills());

    assert!(
        matches!(missing, Err(LintError::MissingPath { .. })),
        "{missing:?}"
    );
}

#[test]
fn suite_kinds_name_their_directories() {
    assert_eq!(
        [SuiteKind::Regression, SuiteKind::Capability].map(SuiteKind::dir),
        ["regression", "capability"]
    );
}
