//! The propose run's diff and `calibrate --apply`, the only write to a thresholds file.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use clap::error::ErrorKind;
use serde_json::json;

use super::{TestResult, calibrate, hand_log};
use crate::calibrate::PROPOSALS_FILE;
use crate::calibrate::output::{PROPOSALS_KIND, PROPOSALS_VERSION};
use crate::cli::Cli;
use crate::decide;
use crate::exit_codes;

/// The D1 ask band's line in the shipped thresholds file.
const ASK_LINE: &str = "            { \"at_least\": 0.4, \"action\": \"ask\" }";

/// The shipped thresholds, copied into `home` as `clef-thresholds.json`.
fn thresholds_copy(home: &Path) -> std::io::Result<PathBuf> {
    let path = home.join("clef-thresholds.json");
    fs::write(&path, decide::thresholds::BUILT_IN)?;
    Ok(path)
}

#[test]
fn the_propose_run_shows_each_threshold_change_as_a_line_diff_and_writes_no_thresholds()
-> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    let thresholds = thresholds_copy(home.path())?;

    let ran = calibrate(home.path(), &["--thresholds", "clef-thresholds.json"]);

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    assert_eq!(
        ran.envelope["data"]["diff"],
        json!([{
            "pointer": "/points/D1/rules/noul/bands/1/at_least",
            "line": 16,
            "before": ASK_LINE,
            "after": ASK_LINE.replace("0.4", "0.3"),
        }])
    );
    assert_eq!(
        fs::read_to_string(&thresholds)?,
        decide::thresholds::BUILT_IN,
        "proposing writes no thresholds"
    );
    assert!(
        ran.stderr
            .lines()
            .any(|line| line.contains("anneal calibrate --apply --thresholds")),
        "{}",
        ran.stderr
    );
    Ok(())
}

#[test]
fn a_proposal_the_scanner_cannot_place_leaves_the_diff_empty_with_its_reason() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    // D1's key written with escapes: strict JSON reads it as `D1`, the scanner compares the
    // key as written and cannot find the pointer.
    let escaped = decide::thresholds::BUILT_IN.replacen("\"D1\":", "\"\\u0044\\u0031\":", 1);
    assert_ne!(
        escaped,
        decide::thresholds::BUILT_IN,
        "the D1 key was escaped"
    );
    let thresholds = home.path().join("clef-thresholds.json");
    fs::write(&thresholds, &escaped)?;

    let ran = calibrate(home.path(), &["--thresholds", "clef-thresholds.json"]);

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    assert_eq!(ran.envelope["data"]["diff"], json!([]));
    let reason = ran.envelope["data"]["diff_error"]
        .as_str()
        .expect("a diff error");
    assert!(
        reason.contains("/points/D1/rules/noul/bands/1/at_least"),
        "{reason}"
    );
    assert_eq!(
        fs::read_to_string(&thresholds)?,
        escaped,
        "proposing writes no thresholds"
    );
    Ok(())
}

#[test]
fn apply_writes_the_proposed_numbers_and_nothing_else() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    let thresholds = thresholds_copy(home.path())?;
    calibrate(home.path(), &["--thresholds", "clef-thresholds.json"]);

    let ran = calibrate(
        home.path(),
        &["--apply", "--thresholds", "clef-thresholds.json"],
    );

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    let expected =
        decide::thresholds::BUILT_IN.replacen(ASK_LINE, &ASK_LINE.replace("0.4", "0.3"), 1);
    assert_eq!(fs::read_to_string(&thresholds)?, expected);
    assert_eq!(ran.envelope["command"], json!("calibrate"));
    let applied = &ran.envelope["data"]["applied"];
    assert_eq!(
        applied,
        &json!([{
            "point": "D1", "question": "noul",
            "pointer": "/points/D1/rules/noul/bands/1/at_least", "from": 0.4, "to": 0.3,
            "line": 16, "before": ASK_LINE, "after": ASK_LINE.replace("0.4", "0.3"),
        }])
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn apply_keeps_the_thresholds_files_permissions() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    let thresholds = thresholds_copy(home.path())?;
    fs::set_permissions(&thresholds, fs::Permissions::from_mode(0o640))?;
    calibrate(home.path(), &["--thresholds", "clef-thresholds.json"]);

    let ran = calibrate(
        home.path(),
        &["--apply", "--thresholds", "clef-thresholds.json"],
    );

    assert_eq!(
        ran.envelope["data"]["written"],
        json!(true),
        "{}",
        ran.envelope
    );
    assert_eq!(
        fs::metadata(&thresholds)?.permissions().mode() & 0o777,
        0o640
    );
    Ok(())
}

#[test]
fn apply_refuses_a_file_that_no_longer_holds_the_proposals_from_and_writes_nothing() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    let thresholds = thresholds_copy(home.path())?;
    calibrate(home.path(), &["--thresholds", "clef-thresholds.json"]);
    let edited_by_hand =
        decide::thresholds::BUILT_IN.replacen(ASK_LINE, &ASK_LINE.replace("0.4", "0.45"), 1);
    fs::write(&thresholds, &edited_by_hand)?;

    let ran = calibrate(
        home.path(),
        &["--apply", "--thresholds", "clef-thresholds.json"],
    );

    assert_eq!(
        ran.code.as_ref().ok(),
        Some(&exit_codes::CALIBRATE_REFUSED.value()),
        "{:?}",
        ran.code
    );
    assert_eq!(ran.envelope["ok"], json!(false));
    assert_eq!(
        ran.envelope["findings"][0]["key"],
        json!("calibrate/refused:/points/D1/rules/noul/bands/1/at_least")
    );
    assert_eq!(
        fs::read_to_string(&thresholds)?,
        edited_by_hand,
        "nothing written"
    );
    Ok(())
}

#[test]
fn apply_with_no_proposals_file_is_a_usage_error() -> TestResult {
    let home = tempfile::tempdir()?;
    thresholds_copy(home.path())?;

    let ran = calibrate(
        home.path(),
        &["--apply", "--thresholds", "clef-thresholds.json"],
    );

    let code = ran.code.as_ref().map_err(|error| error.code().value());
    assert_eq!(code, Err(exit_codes::USAGE.value()), "{:?}", ran.code);
    assert_eq!(ran.envelope, serde_json::Value::Null, "nothing on stdout");
    Ok(())
}

#[test]
fn apply_with_point_writes_only_that_points_steps() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    let thresholds = thresholds_copy(home.path())?;
    calibrate(home.path(), &["--thresholds", "clef-thresholds.json"]);

    let ran = calibrate(
        home.path(),
        &[
            "--apply",
            "--point",
            "D7",
            "--thresholds",
            "clef-thresholds.json",
        ],
    );

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    assert_eq!(
        (
            &ran.envelope["data"]["applied"],
            &ran.envelope["data"]["written"]
        ),
        (&json!([]), &json!(false))
    );
    assert_eq!(
        fs::read_to_string(&thresholds)?,
        decide::thresholds::BUILT_IN,
        "D1's step is not D7's"
    );
    Ok(())
}

#[test]
fn a_proposals_file_of_another_kind_is_refused_as_an_internal_error() -> TestResult {
    let home = tempfile::tempdir()?;
    thresholds_copy(home.path())?;
    fs::create_dir_all(home.path().join("decisions"))?;
    fs::write(
        home.path().join("decisions").join("proposals.json"),
        r#"{"kind": "something-else", "version": 1, "apply": []}"#,
    )?;

    let ran = calibrate(
        home.path(),
        &["--apply", "--thresholds", "clef-thresholds.json"],
    );

    let code = ran.code.as_ref().map_err(|error| error.code().value());
    assert_eq!(code, Err(exit_codes::INTERNAL.value()), "{:?}", ran.code);
    Ok(())
}

#[test]
fn apply_needs_an_explicit_thresholds_file() {
    let parsed = Cli::try_parse_from(["anneal", "calibrate", "--apply"]);

    assert_eq!(
        parsed.map(|_cli| ()).map_err(|error| error.kind()),
        Err(ErrorKind::MissingRequiredArgument),
        "--apply without --thresholds is refused"
    );
}

/// Writes a proposals file holding one `noul` threshold step of `point` at `pointer`, moving
/// it `(from, to)`.
fn proposals_with_step(
    home: &Path,
    point: &str,
    pointer: &str,
    (from, to): (f64, f64),
) -> TestResult {
    let dir = home.join("decisions");
    fs::create_dir_all(&dir)?;
    let file = json!({
        "kind": PROPOSALS_KIND,
        "version": PROPOSALS_VERSION,
        "apply": [{
            "kind": "threshold", "point": point, "question": "noul",
            "pointer": pointer, "from": from, "to": to,
        }],
    });
    fs::write(dir.join(PROPOSALS_FILE), serde_json::to_vec_pretty(&file)?)?;
    Ok(())
}

/// Runs `--apply` with `extra` words and checks the run was refused with `key`, the thresholds
/// file left byte for byte as shipped.
fn assert_refused(home: &Path, thresholds: &Path, extra: &[&str], key: &str) -> TestResult {
    let mut words = vec!["--apply", "--thresholds", "clef-thresholds.json"];
    words.extend_from_slice(extra);

    let ran = calibrate(home, &words);

    assert_eq!(
        ran.code.as_ref().ok(),
        Some(&exit_codes::CALIBRATE_REFUSED.value()),
        "{:?}",
        ran.code
    );
    assert_eq!(
        (
            &ran.envelope["findings"][0]["key"],
            &ran.envelope["data"]["written"]
        ),
        (&json!(key), &json!(false)),
        "{}",
        ran.envelope
    );
    assert_eq!(
        fs::read(thresholds)?,
        decide::thresholds::BUILT_IN.as_bytes(),
        "nothing written"
    );
    Ok(())
}

#[test]
fn apply_refuses_steps_that_would_not_read_as_strict_thresholds() -> TestResult {
    let home = tempfile::tempdir()?;
    let thresholds = thresholds_copy(home.path())?;
    // D1's ask band raised above its deny band (0.9): the bands would no longer descend.
    proposals_with_step(
        home.path(),
        "D1",
        "/points/D1/rules/noul/bands/1/at_least",
        (0.4, 0.95),
    )?;

    assert_refused(
        home.path(),
        &thresholds,
        &[],
        "calibrate/refused:thresholds",
    )
}

#[test]
fn apply_refuses_a_pointer_that_does_not_read_back_its_new_value() -> TestResult {
    let home = tempfile::tempdir()?;
    let thresholds = thresholds_copy(home.path())?;
    // The scanner reads the index `01` as 1; a JSON pointer allows no leading zero, so the
    // strict read-back finds nothing there.
    let pointer = "/points/D1/rules/noul/bands/01/at_least";
    proposals_with_step(home.path(), "D1", pointer, (0.4, 0.3))?;

    assert_refused(
        home.path(),
        &thresholds,
        &[],
        &format!("calibrate/refused:{pointer}"),
    )
}

#[test]
fn apply_refuses_a_step_whose_pointer_is_not_under_its_own_point() -> TestResult {
    let home = tempfile::tempdir()?;
    let thresholds = thresholds_copy(home.path())?;
    // A D7 step, selected by `--point D7`, that would edit D1's ask band.
    let pointer = "/points/D1/rules/noul/bands/1/at_least";
    proposals_with_step(home.path(), "D7", pointer, (0.4, 0.3))?;

    assert_refused(
        home.path(),
        &thresholds,
        &["--point", "D7"],
        &format!("calibrate/refused:{pointer}"),
    )
}
