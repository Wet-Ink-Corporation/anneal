//! What the cockpit's calibration pane reads: a run that changes nothing leaves the proposals
//! file alone, and each point carries its mode and its history (#158).

use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::{TestResult, args, calibrate, hand_log, label, point};
use crate::calibrate::{CalibrateEnv, PROPOSALS_FILE, run};
use crate::decide;

/// A modification time long past, so a rewrite of the file shows.
fn age(path: &Path) -> Result<SystemTime, Box<dyn Error>> {
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    File::options().write(true).open(path)?.set_modified(old)?;
    Ok(old)
}

/// Runs `anneal calibrate` with `home` as the anneal home and `vars` set; returns the envelope.
fn calibrate_with(home: &Path, vars: &[(&str, &str)]) -> Result<Value, Box<dyn Error>> {
    let env = CalibrateEnv {
        cwd: home,
        lookup: |var: &str| -> Option<OsString> {
            if var == "ANNEAL_HOME" {
                return Some(home.as_os_str().to_owned());
            }
            vars.iter()
                .find(|(name, _)| *name == var)
                .map(|(_, value)| OsString::from(value))
        },
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&args(&[]), &env, &mut out, &mut err)?;
    assert_eq!(code.value(), 0);
    Ok(serde_json::from_slice(&out)?)
}

/// Each point's `mode` in the envelope, D1 to D7.
fn modes(envelope: &Value) -> Vec<&Value> {
    envelope["data"]["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|point| &point["mode"])
        .collect()
}

/// `counts.json` as `decide` keeps it, with `answered` decisions for D1.
fn d1_answered(home: &Path, answered: u64) -> io::Result<()> {
    let counts = json!({
        "kind": "anneal-decision-counts", "version": 1, "answered": {"D1": answered},
    });
    fs::write(
        home.join("decisions").join("counts.json"),
        counts.to_string(),
    )
}

#[test]
fn without_an_account_every_point_is_off() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let envelope = calibrate_with(home.path(), &[])?;

    assert_eq!(modes(&envelope), [&json!("off"); 7]);
    assert_eq!(envelope["data"].get("mode_error"), None);
    Ok(())
}

#[test]
fn each_point_carries_the_mode_in_effect_as_status_reports_it() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    d1_answered(home.path(), 50)?;
    let vars = [
        (decide::config::ACCOUNT_VAR, "acct"),
        (decide::config::MODE_VAR, "enforce,D7=off"),
    ];

    let envelope = calibrate_with(home.path(), &vars)?;

    // D1 has its 50 answered decisions; D2 to D6 are held in shadow until they do.
    let shadow = json!("shadow");
    assert_eq!(
        modes(&envelope),
        [
            &json!("enforce"),
            &shadow,
            &shadow,
            &shadow,
            &shadow,
            &shadow,
            &json!("off")
        ]
    );
    Ok(())
}

#[test]
fn a_mode_that_cannot_be_read_is_null_with_the_reason_and_the_run_still_succeeds() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let envelope = calibrate_with(home.path(), &[(decide::config::MODE_VAR, "sometimes")])?;

    assert_eq!(modes(&envelope), [&Value::Null; 7]);
    let reason = envelope["data"]["mode_error"].as_str().expect("a reason");
    assert!(
        reason.contains("CLAUDE_PLUGIN_OPTION_CLEF_MODE"),
        "{reason}"
    );
    Ok(())
}

#[test]
fn a_run_that_would_write_the_same_proposals_leaves_the_file_alone() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    calibrate(home.path(), &[]);
    let proposals = home.path().join("decisions").join(PROPOSALS_FILE);
    let before = fs::read(&proposals)?;
    let old = age(&proposals)?;

    let ran = calibrate(home.path(), &[]);

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    assert_eq!(fs::metadata(&proposals)?.modified()?, old, "not rewritten");
    assert_eq!(fs::read(&proposals)?, before);
    Ok(())
}

#[test]
fn a_run_whose_proposals_change_rewrites_the_file() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    calibrate(home.path(), &[]);
    let proposals = home.path().join("decisions").join(PROPOSALS_FILE);
    let old = age(&proposals)?;
    // Relabelling the decision at 0.3 makes 0.3 no longer the cheapest threshold.
    let live = home.path().join("decisions").join("decisions.jsonl");
    let mut rows = fs::read_to_string(&live)?;
    rows.push_str(&(label("d1-1-1", "no").to_string() + "\n"));
    fs::write(&live, rows)?;

    let ran = calibrate(home.path(), &[]);

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    assert_ne!(fs::metadata(&proposals)?.modified()?, old, "rewritten");
    Ok(())
}

#[test]
fn each_point_carries_its_history_per_day_of_decisions() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let ran = calibrate(home.path(), &[]);

    // Every decision of the hand set has `ts_ms` 1: the first UTC day of 1970.
    let d1 = &point(&ran.envelope, "D1")["history"];
    assert_eq!(
        (&d1[0]["question"], &d1[0]["day_start_ms"], &d1[0]["n"]),
        (&json!("noul"), &json!(0), &json!(5))
    );
    assert_eq!(d1[0]["labelled"], json!(5));
    assert_eq!(
        d1[0]["brier"],
        point(&ran.envelope, "D1")["questions"][0]["brier"]
    );
    assert_eq!(d1.as_array().map(Vec::len), Some(1));
    assert_eq!(point(&ran.envelope, "D2")["history"], json!([]));
    Ok(())
}

#[test]
fn the_proposals_file_carries_neither_mode_nor_history() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    calibrate(home.path(), &[]);

    let proposals = home.path().join("decisions").join(PROPOSALS_FILE);
    let file: Value = serde_json::from_slice(&fs::read(proposals)?)?;
    let d1 = &file["points"][0];
    assert_eq!((d1.get("mode"), d1.get("history")), (None, None), "{d1}");
    Ok(())
}
