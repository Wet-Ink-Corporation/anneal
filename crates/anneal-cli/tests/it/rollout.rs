//! `anneal rollout decide` through the built binary, on the band fixtures in
//! `tests/fixtures/rollout/`: metrics exactly at each boundary exit 0 go, 1 hold, 2 rollback.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rollout")
}

/// `rollout decide` with the metrics of the fixture `metrics` and the fixture bands.
fn decide(metrics: &str) -> Result<Output, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let metrics = std::fs::read_to_string(fixtures().join(metrics))?;
    let bands = fixtures().join("bands.json");
    Ok(anneal_in(dir.path())
        .args(["rollout", "decide", "--metrics", metrics.trim()])
        .arg("--bands")
        .arg(bands)
        .output()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn metrics_just_inside_every_go_band_exit_0_go() -> TestResult {
    let output = decide("go.json")?;
    let envelope = envelope(&output)?;

    assert_eq!(output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (&envelope["ok"], &envelope["data"]["decision"]),
        (&json!(true), &json!("go")),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn metrics_exactly_at_every_hold_threshold_exit_1_hold() -> TestResult {
    let output = decide("hold.json")?;
    let envelope = envelope(&output)?;

    assert_eq!(output.status.code(), Some(1), "{envelope}");
    assert_eq!(&envelope["data"]["decision"], &json!("hold"), "{envelope}");
    let keys: Vec<&str> = envelope["findings"]
        .as_array()
        .map(|findings| findings.iter().filter_map(|f| f["key"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "rollout/hold:error_rate",
            "rollout/hold:p99_ms",
            "rollout/hold:success_rate"
        ],
        "one finding per holding metric"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn metrics_exactly_at_every_rollback_threshold_exit_2_rollback() -> TestResult {
    let output = decide("rollback.json")?;
    let envelope = envelope(&output)?;

    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(
        (&envelope["ok"], &envelope["data"]["decision"]),
        (&json!(false), &json!("rollback")),
        "a rollback comes with its envelope, unlike a usage error"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_bands_file_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = anneal_in(dir.path())
        .args([
            "rollout",
            "decide",
            "--metrics",
            "{}",
            "--bands",
            "nope.json",
        ])
        .output()?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope for a usage error");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR rollout: bands file nope.json") && stderr.lines().count() == 1,
        "one ERROR line naming the file: {stderr}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn metrics_that_are_not_an_object_of_numbers_are_a_usage_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    let bands = fixtures().join("bands.json");

    let output = anneal_in(dir.path())
        .args(["rollout", "decide", "--metrics", r#"{"error_rate": "low"}"#])
        .arg("--bands")
        .arg(bands)
        .output()?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope for a usage error");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn metrics_naming_a_metric_twice_are_a_usage_error_naming_it() -> TestResult {
    // Without the check the later value would silently win: here, the go one.
    let dir = tempfile::tempdir()?;
    let bands = fixtures().join("bands.json");

    let output = anneal_in(dir.path())
        .args([
            "rollout",
            "decide",
            "--metrics",
            r#"{"error_rate": 0.2, "error_rate": 0.001}"#,
        ])
        .arg("--bands")
        .arg(bands)
        .output()?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope for a usage error");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR rollout: --metrics")
            && stderr.contains("metric \"error_rate\" is given twice")
            && stderr.lines().count() == 1,
        "one ERROR line naming the metric: {stderr}"
    );
    Ok(())
}
