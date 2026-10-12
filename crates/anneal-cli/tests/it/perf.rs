//! `anneal perf sample|noise` through the built binary with a real command (`git --version`,
//! on every CI platform). Nothing here depends on how long a run takes: the keep or revert
//! decision itself is pinned by the fake-runner tests in `perf::tests`.

use std::error::Error;
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and git")]
fn sample_saves_n_runs_and_noise_compares_against_them() -> TestResult {
    let dir = tempfile::tempdir()?;

    let sampled = anneal_in(dir.path())
        .args(["perf", "sample", "-n", "3", "--out", "base.json", "--"])
        .args(["git", "--version"])
        .output()?;
    let saved: Value = serde_json::from_slice(&std::fs::read(dir.path().join("base.json"))?)?;
    let noise = anneal_in(dir.path())
        .args(["perf", "noise", "-n", "3", "--baseline", "base.json", "--"])
        .args(["git", "--version"])
        .output()?;
    let compared = envelope(&noise)?;

    assert_eq!(sampled.status.code(), Some(0), "{sampled:?}");
    assert_eq!(
        (
            &saved["version"],
            &saved["command"],
            saved["samples_secs"].as_array().map(Vec::len)
        ),
        (&json!(1), &json!(["git", "--version"]), Some(3)),
        "the sample file"
    );
    let code = noise.status.code();
    assert!(matches!(code, Some(0 | 1)), "keep or revert: {noise:?}");
    assert_eq!(
        (&compared["ok"], &compared["data"]["candidate"]["runs"]),
        (&json!(code == Some(0)), &json!(3)),
        "the envelope agrees with the exit code: {compared}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and git")]
fn a_run_that_fails_is_an_internal_error_naming_the_run_and_saves_nothing() -> TestResult {
    // git exits 129 on an unknown option: the run started and failed, so its time measures
    // nothing.
    let dir = tempfile::tempdir()?;

    let output = anneal_in(dir.path())
        .args(["perf", "sample", "-n", "2", "--out", "x", "--"])
        .args(["git", "--no-such-option"])
        .output()?;

    assert_eq!(output.status.code(), Some(70), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope");
    assert!(!dir.path().join("x").exists(), "nothing saved");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR perf: run 1 of 2: the command failed (exit 129)")
            && stderr.lines().count() == 1,
        "one ERROR line naming the run: {stderr}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_command_that_cannot_start_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = anneal_in(dir.path())
        .args(["perf", "sample", "-n", "2", "--out", "base.json", "--"])
        .arg("anneal-no-such-program")
        .output()?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope");
    assert!(!dir.path().join("base.json").exists(), "nothing saved");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR perf: could not start anneal-no-such-program")
            && stderr.lines().count() == 1,
        "one ERROR line: {stderr}"
    );
    Ok(())
}
