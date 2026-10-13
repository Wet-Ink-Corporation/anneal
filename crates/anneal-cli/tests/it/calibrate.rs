//! `anneal calibrate` through the built binary, over the hand-computed decision log in
//! `tests/fixtures/calibrate/` (its README works the numbers through).

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

use crate::support::{anneal_in, copy_dir};

type TestResult = Result<(), Box<dyn Error>>;

/// `anneal` in `home` with `home` as the anneal home and no plugin root.
fn anneal(home: &Path) -> Command {
    let mut command = anneal_in(home);
    command
        .env("ANNEAL_HOME", home)
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("CLAUDE_PLUGIN_OPTION_CLEF_MODE");
    command
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("calibrate")
        .join("decisions")
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn close(value: &Value, expected: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|actual| (actual - expected).abs() < 1e-12)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_hand_computed_log_gives_its_brier_bins_ask_rate_and_proposal() -> TestResult {
    let home = tempfile::tempdir()?;
    copy_dir(&fixture(), &home.path().join("decisions"))?;

    let output = anneal(home.path())
        .args(["calibrate", "--point", "D1"])
        .output()?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope = envelope(&output)?;
    let data = &envelope["data"];
    assert_eq!(
        (&data["skipped_rows"], &data["unmatched_labels"]),
        (&json!(1), &json!(1)),
        "the torn line; the label for no decision"
    );
    assert_eq!(data["files"].as_array().map(Vec::len), Some(2));
    let noul = &data["points"][0]["questions"][0];
    assert_eq!(
        [
            &noul["n"],
            &noul["labelled"],
            &noul["unanswered"],
            &noul["unusable_labels"],
            &noul["asks"]
        ],
        [&json!(6), &json!(5), &json!(1), &json!(1), &json!(2)]
    );
    assert!(close(&noul["brier"], 0.2885), "{noul}");
    assert!(close(&noul["ask_rate"], 1.0 / 3.0), "{noul}");
    let bins = noul["bins"].as_array().expect("bins");
    let counts: Vec<u64> = bins
        .iter()
        .filter_map(|bin| bin["count"].as_u64())
        .collect();
    assert_eq!(
        counts,
        [0, 1, 0, 1, 0, 0, 0, 0, 1, 2],
        "0.3 is in [0.3, 0.4)"
    );
    let last = &bins[9];
    assert!(close(&last["mean_probability"], 0.975), "{last}");
    assert!(close(&last["observed_frequency"], 0.5), "{last}");
    assert_eq!(bins[0]["observed_frequency"], Value::Null, "an empty bin");
    let threshold = &noul["threshold"];
    assert_eq!(
        (&threshold["current"], &threshold["proposed"]),
        (&json!(0.4), &json!(0.3))
    );
    assert!(
        close(&threshold["current_expected_cost"], 1.2),
        "{threshold}"
    );
    assert!(
        close(&threshold["proposed_expected_cost"], 0.2),
        "{threshold}"
    );
    assert_eq!(data["points"][0]["verdict"], json!("stay-shadow"));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_short_summary_goes_to_stderr_and_the_proposals_to_the_anneal_home() -> TestResult {
    let home = tempfile::tempdir()?;
    copy_dir(&fixture(), &home.path().join("decisions"))?;

    let output = anneal(home.path()).arg("calibrate").output()?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stderr = std::str::from_utf8(&output.stderr)?;
    assert!(stderr.lines().count() <= 20, "short:\n{stderr}");
    assert!(!stderr.contains("ERROR"), "{stderr}");
    assert!(
        stderr.lines().any(|line| line == "D1: stay-shadow"),
        "{stderr}"
    );
    let proposals = home.path().join("decisions").join("proposals.json");
    let file: Value = serde_json::from_slice(&fs::read(&proposals)?)?;
    assert_eq!(file["points"].as_array().map(Vec::len), Some(7));
    assert_eq!(
        envelope(&output)?["data"]["proposals"],
        json!(proposals.to_str().expect("a UTF-8 temporary path"))
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn only_apply_writes_the_proposed_threshold_into_the_file_it_names() -> TestResult {
    let home = tempfile::tempdir()?;
    copy_dir(&fixture(), &home.path().join("decisions"))?;
    let shipped =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/playbooks/clef-thresholds.json");
    let thresholds = home.path().join("clef-thresholds.json");
    fs::copy(&shipped, &thresholds)?;
    let before = fs::read_to_string(&thresholds)?;

    let proposed = anneal(home.path())
        .args(["calibrate", "--point", "D1", "--thresholds"])
        .arg(&thresholds)
        .output()?;
    let unchanged = fs::read_to_string(&thresholds)?;
    let applied = anneal(home.path())
        .args(["calibrate", "--apply", "--point", "D1", "--thresholds"])
        .arg(&thresholds)
        .output()?;

    assert_eq!(proposed.status.code(), Some(0), "{proposed:?}");
    assert_eq!(unchanged, before, "proposing writes no thresholds");
    let diff = &envelope(&proposed)?["data"]["diff"];
    assert_eq!(applied.status.code(), Some(0), "{applied:?}");
    assert_eq!(
        envelope(&applied)?["data"]["applied"][0]["after"],
        diff[0]["after"],
        "apply writes what the diff showed"
    );
    let after = fs::read_to_string(&thresholds)?;
    let changed: Vec<(&str, &str)> = before
        .lines()
        .zip(after.lines())
        .filter(|(old, new)| old != new)
        .collect();
    assert_eq!(
        changed,
        [(
            "            { \"at_least\": 0.4, \"action\": \"ask\" }",
            "            { \"at_least\": 0.3, \"action\": \"ask\" }"
        )],
        "one number changed, the layout kept"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_point_with_49_labelled_decisions_is_never_eligible_to_enforce() -> TestResult {
    let home = tempfile::tempdir()?;
    let dir = home.path().join("decisions");
    fs::create_dir_all(&dir)?;
    let mut log = String::new();
    for k in 0..49 {
        let positive = k % 2 == 0;
        let probability = if positive { 1.0 } else { 0.0 };
        let id = format!("d1760000000000-7-{k}");
        let decision = json!({
            "kind": "decision", "id": id, "point": "D7", "question": "noul",
            "probabilities": {"yes": probability, "no": 1.0 - probability},
            "top": null, "probability": probability, "action": "find_fact",
        });
        let label =
            json!({"kind": "label", "id": id, "outcome": if positive { "yes" } else { "no" }});
        writeln!(log, "{decision}\n{label}")?;
    }
    fs::write(dir.join("decisions.jsonl"), log)?;

    let output = anneal(home.path())
        .args(["calibrate", "--point", "D7"])
        .output()?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let d7 = &envelope(&output)?["data"]["points"][0];
    assert_eq!(d7["questions"][0]["labelled"], json!(49));
    assert_eq!(
        d7["questions"][0]["brier"],
        json!(0.0),
        "perfectly calibrated"
    );
    assert_eq!(d7["verdict"], json!("stay-shadow"), "49 is under 50");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_current_threshold_is_kept_when_no_candidate_is_cheaper() -> TestResult {
    // D1 decisions at 0.1, 0.15 and 0.2, all labelled no: the current 0.4 asks nothing
    // needlessly, and every observed probability would.
    let home = tempfile::tempdir()?;
    let dir = home.path().join("decisions");
    fs::create_dir_all(&dir)?;
    let mut log = String::new();
    for (k, probability) in [0.1, 0.15, 0.2].into_iter().enumerate() {
        let id = format!("d1760000000000-1-{k}");
        let decision = json!({
            "kind": "decision", "id": id, "point": "D1", "question": "noul",
            "probabilities": {"yes": probability, "no": 1.0 - probability},
            "top": null, "probability": probability, "action": "defer",
        });
        let label = json!({"kind": "label", "id": id, "outcome": "no"});
        writeln!(log, "{decision}\n{label}")?;
    }
    fs::write(dir.join("decisions.jsonl"), log)?;

    let output = anneal(home.path())
        .args(["calibrate", "--point", "D1"])
        .output()?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope = envelope(&output)?;
    let threshold = &envelope["data"]["points"][0]["questions"][0]["threshold"];
    assert_eq!(
        (&threshold["current"], &threshold["proposed"]),
        (&json!(0.4), &json!(0.4)),
        "{threshold}"
    );
    assert_eq!(
        threshold["proposed_expected_cost"],
        json!(0.0),
        "{threshold}"
    );
    assert_eq!(envelope["data"]["apply"], json!([]), "nothing to apply");
    Ok(())
}
