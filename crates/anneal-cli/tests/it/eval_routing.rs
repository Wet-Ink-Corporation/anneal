//! `anneal eval routing` against the fixture sets in `tests/fixtures/eval-routing/`: `skills/`
//! holds three model-invoked skills and one user-invoked one; `pass/` routes every prompt
//! correctly, `fail/` has one positive that ranks the wrong skill first.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval-routing")
}

/// Runs `anneal eval routing --skills <skills> --fixtures <routing>` and returns the exit
/// code, the parsed envelope (`Null` when stdout is empty) and stderr.
fn eval_routing(
    skills: &Path,
    routing: &Path,
) -> Result<(Option<i32>, Value, String), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(["eval", "routing", "--skills"])
        .arg(skills)
        .arg("--fixtures")
        .arg(routing)
        .output()?;
    let envelope = if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)?
    };
    Ok((
        output.status.code(),
        envelope,
        String::from_utf8(output.stderr)?,
    ))
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_passing_set_exits_0_and_its_table_is_all_diagonal() -> TestResult {
    let (code, envelope, stderr) =
        eval_routing(&fixtures().join("skills"), &fixtures().join("pass"))?;

    assert_eq!(code, Some(0), "100% rank-1: {envelope}");
    assert_eq!(stderr, "", "nothing on stderr");
    assert_eq!(
        envelope["command"],
        json!("eval routing"),
        "the subcommand is named"
    );
    assert_eq!(
        (
            &envelope["data"]["skills"],
            &envelope["data"]["cases"],
            &envelope["data"]["failed"]
        ),
        (&json!(3), &json!(48), &json!(0)),
        "three routable skills (release is user-invoked), 16 prompts each"
    );
    assert_eq!(
        envelope["data"]["table"],
        json!([
            "expected \\ top  diagnose  plan  spec",
            "diagnose              16     0     0",
            "plan                   0    16     0",
            "spec                   0     0    16",
        ]),
        "rows are expected skills, columns the top-ranked skill"
    );
    assert_eq!(envelope["findings"], json!([]), "no finding");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_failing_set_exits_1_and_the_table_shows_the_confusion() -> TestResult {
    let (code, envelope, _) = eval_routing(&fixtures().join("skills"), &fixtures().join("fail"))?;

    assert_eq!(code, Some(1), "below bound: {envelope}");
    assert_eq!(envelope["ok"], json!(false), "not ok");
    assert_eq!(
        envelope["data"]["confusion"]["plan"],
        json!({"diagnose": 0, "plan": 15, "spec": 1}),
        "one plan prompt ranks spec first"
    );
    let findings = envelope["findings"]
        .as_array()
        .ok_or("findings is an array")?;
    let [finding] = findings.as_slice() else {
        return Err(format!("one finding, got {findings:?}").into());
    };
    let key = finding["key"].as_str().ok_or("key is a string")?;
    assert!(
        key.starts_with("eval/routing/positive:") && key.ends_with("plan.yaml:10"),
        "the key names the kind, file and line: {key}"
    );
    assert_eq!(
        finding["message"],
        json!(
            "positive for `plan` ranked `spec` first (plan 1, spec 3): \"turn this rough idea into a plan\""
        ),
        "the message names both skills, their scores and the prompt"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_invalid_fixture_exits_2_with_an_envelope() -> TestResult {
    let work = tempfile::tempdir()?;
    for name in ["spec.yaml", "diagnose.yaml"] {
        std::fs::copy(fixtures().join("pass").join(name), work.path().join(name))?;
    }
    std::fs::write(
        work.path().join("plan.yaml"),
        "positives:\n  - [not, a, prompt]\n",
    )?;

    let (code, envelope, stderr) = eval_routing(&fixtures().join("skills"), work.path())?;

    assert_eq!(code, Some(2), "invalid fixtures: {envelope}");
    assert_eq!(stderr, "", "a command result, not an error");
    let keys: Vec<&str> = envelope["findings"]
        .as_array()
        .ok_or("findings is an array")?
        .iter()
        .filter_map(|f| f["key"].as_str())
        .collect();
    assert_eq!(keys.len(), 1, "one problem: {keys:?}");
    assert!(
        keys.iter()
            .all(|k| k.starts_with("eval/routing/fixture:") && k.ends_with("plan.yaml:2")),
        "the parse error's file and line: {keys:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_fixtures_dir_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let (code, envelope, stderr) =
        eval_routing(&fixtures().join("skills"), &fixtures().join("no-such-dir"))?;

    assert_eq!(code, Some(2), "usage error");
    assert_eq!(envelope, Value::Null, "nothing on stdout");
    assert!(
        stderr.starts_with("ERROR eval: reading fixtures: no such path: ")
            && stderr.lines().count() == 1,
        "one ERROR line: {stderr:?}"
    );
    Ok(())
}
