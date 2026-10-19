use std::fs;

use serde_json::{Value, json};

use super::{Answered, HEADROOM_ROWS, MAX_LOG_FILE_BYTES, ROW_BYTES, read_log};
use crate::baseline::BaselineError;
use crate::calibrate::CalibrateError;
use crate::decide::log::{DecisionLog, MAX_FILE_BYTES, MAX_ROW_BYTES};
use crate::decide::point::{Point, Question};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn decision(id: &str, probability: Option<f64>, action: Option<&str>) -> Value {
    json!({
        "kind": "decision", "id": id, "ts_ms": 1, "point": "D1", "question": "noul",
        "subject": "destructive_or_irreversible", "model": "m", "state_sha256": "sha256:x",
        "probabilities": probability.map(|p| json!({"yes": p, "no": 1.0 - p})),
        "top": null, "probability": probability, "action": action, "mode": "shadow",
        "configured_mode": "shadow", "latency_ms": 1, "outcome": null,
        "unreachable": probability.is_none(), "error": null, "error_reason": null,
    })
}

fn label(id: &str, outcome: &str) -> Value {
    json!({"kind": "label", "id": id, "outcome": outcome, "ts_ms": 2})
}

fn write_lines(path: &std::path::Path, rows: &[Value], extra: &str) -> std::io::Result<()> {
    let mut text: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    text.push_str(extra);
    fs::write(path, text)
}

/// A D1 noul decision as the reader returns it.
fn answered(id: &str, probability: f64, asked: bool) -> Answered {
    Answered {
        id: id.to_owned(),
        point: Point::D1,
        question: Question::Noul,
        probability,
        top: None,
        options: vec!["no".to_owned(), "yes".to_owned()],
        asked,
    }
}

#[test]
fn rows_are_read_from_the_rotated_files_then_the_live_one_and_the_last_label_wins() -> TestResult {
    let home = tempfile::tempdir()?;
    let dir = home.path().join("decisions");
    fs::create_dir_all(&dir)?;
    write_lines(
        &dir.join("decisions.2.jsonl"),
        &[
            decision("d1-1-0", Some(0.25), Some("defer")),
            label("d1-1-0", "yes"),
        ],
        "",
    )?;
    write_lines(
        &dir.join("decisions.1.jsonl"),
        &[decision("d1-1-1", None, None), label("d1-1-0", "no")],
        "",
    )?;
    write_lines(
        &dir.join("decisions.jsonl"),
        &[decision("d1-1-2", Some(0.5), Some("ask"))],
        "not json\n{\"kind\": \"other\"}\n",
    )?;

    let rows = read_log(&DecisionLog::in_home(home.path()))?;

    assert_eq!(
        rows.answered,
        [
            answered("d1-1-0", 0.25, false),
            answered("d1-1-2", 0.5, true)
        ]
    );
    assert_eq!(
        rows.labels.get("d1-1-0").map(String::as_str),
        Some("no"),
        "later wins"
    );
    assert_eq!(rows.unanswered.get(&(Point::D1, Question::Noul)), Some(&1));
    assert_eq!(
        rows.skipped_rows, 2,
        "a line that is not JSON and an unknown kind"
    );
    assert_eq!(
        rows.files.len(),
        3,
        "decisions.2, decisions.1 and the live file"
    );
    Ok(())
}

#[test]
fn a_probability_outside_0_to_1_is_a_skipped_row() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());
    fs::create_dir_all(home.path().join("decisions"))?;
    write_lines(
        &log.path(),
        &[decision("d1-1-0", Some(1.5), Some("deny"))],
        "",
    )?;

    let rows = read_log(&log)?;

    assert_eq!((rows.answered.len(), rows.skipped_rows), (0, 1));
    Ok(())
}

#[test]
fn a_negative_zero_probability_is_read_as_zero() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());
    fs::create_dir_all(home.path().join("decisions"))?;
    write_lines(
        &log.path(),
        &[decision("d1-1-0", Some(-0.0), Some("defer"))],
        "",
    )?;

    let rows = read_log(&log)?;

    let probability = rows.answered.first().expect("one answered row").probability;
    assert!(
        probability.total_cmp(&0.0).is_eq(),
        "{probability:?} groups with 0.0 under total_cmp"
    );
    Ok(())
}

#[test]
fn no_log_reads_as_no_rows() -> TestResult {
    let home = tempfile::tempdir()?;

    let rows = read_log(&DecisionLog::in_home(home.path()))?;

    assert_eq!(
        (rows.answered.len(), rows.labels.len(), rows.files.len()),
        (0, 0, 0)
    );
    Ok(())
}

#[test]
fn a_directory_where_a_log_file_should_be_is_an_error() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());
    fs::create_dir_all(log.path())?;

    let result = read_log(&log);

    assert!(
        matches!(
            result,
            Err(CalibrateError::Read(BaselineError::NotAFile { .. }))
        ),
        "{result:?}"
    );
    Ok(())
}

/// Writes a live log of exactly `size` bytes: one decision row, then blank lines.
fn live_log_of(log: &DecisionLog, size: u64) -> TestResult {
    fs::create_dir_all(log.dir())?;
    let mut bytes = decision("d1-1-0", Some(0.5), Some("ask"))
        .to_string()
        .into_bytes();
    bytes.push(b'\n');
    bytes.resize(usize::try_from(size)?, b'\n');
    fs::write(log.path(), bytes)?;
    Ok(())
}

#[test]
fn the_headroom_is_64_rows_of_decides_row_cap() {
    assert_eq!(
        u64::try_from(MAX_ROW_BYTES),
        Ok(ROW_BYTES),
        "decide's row cap"
    );
    assert_eq!(
        MAX_LOG_FILE_BYTES,
        MAX_FILE_BYTES + HEADROOM_ROWS * ROW_BYTES
    );
}

#[test]
fn a_live_file_just_past_the_rotation_size_is_still_read() -> TestResult {
    // Unlocked concurrent appends can each pass decide's check and land past 16 MiB.
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());
    live_log_of(&log, MAX_FILE_BYTES + 100 * 1024)?;

    let rows = read_log(&log)?;

    assert_eq!(rows.answered.len(), 1, "the row before the padding is read");
    Ok(())
}

#[test]
fn a_file_past_the_headroom_is_an_error() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());
    live_log_of(&log, MAX_LOG_FILE_BYTES + 1)?;

    let result = read_log(&log);

    assert!(
        matches!(
            &result,
            Err(CalibrateError::TooLarge { path, cap })
                if *path == log.path() && *cap == MAX_LOG_FILE_BYTES
        ),
        "{result:?}"
    );
    Ok(())
}

/// Writes `rows` as the live log of a fresh home and reads it back.
fn read_rows(rows: &[Value]) -> Result<super::LogRows, Box<dyn std::error::Error>> {
    let home = tempfile::tempdir()?;
    let dir = home.path().join("decisions");
    fs::create_dir_all(&dir)?;
    write_lines(&dir.join("decisions.jsonl"), rows, "")?;
    Ok(read_log(&DecisionLog::in_home(home.path()))?)
}

#[test]
fn a_label_row_without_actor_or_via_still_reads_and_is_not_via_mod() -> TestResult {
    let rows = read_rows(&[label("d1-1-0", "yes")])?;

    assert_eq!(
        (
            rows.labels.get("d1-1-0").map(String::as_str),
            rows.via_mod.len(),
            rows.skipped_rows
        ),
        (Some("yes"), 0, 0)
    );
    Ok(())
}

#[test]
fn a_via_this_build_does_not_know_still_reads_as_a_label() -> TestResult {
    let rows = read_rows(&[json!({
        "kind": "label", "id": "d1-1-0", "outcome": "no", "ts_ms": 2,
        "actor": "robot", "via": "carrier-pigeon",
    })])?;

    assert_eq!(
        (
            rows.labels.get("d1-1-0").map(String::as_str),
            rows.via_mod.len(),
            rows.skipped_rows
        ),
        (Some("no"), 0, 0)
    );
    Ok(())
}

#[test]
fn the_last_label_of_an_id_decides_whether_it_came_via_the_mod() -> TestResult {
    let via = |id: &str, outcome: &str, via: &str| json!({"kind": "label", "id": id, "outcome": outcome, "ts_ms": 2, "via": via});

    let rows = read_rows(&[
        label("d1-1-0", "no"),
        via("d1-1-0", "yes", "mod"),
        via("d1-1-1", "yes", "mod"),
        via("d1-1-1", "no", "terminal"),
    ])?;

    assert_eq!(
        (
            rows.labels.get("d1-1-0").map(String::as_str),
            rows.labels.get("d1-1-1").map(String::as_str)
        ),
        (Some("yes"), Some("no")),
        "the later label wins"
    );
    assert_eq!(
        rows.via_mod.iter().map(String::as_str).collect::<Vec<_>>(),
        ["d1-1-0"],
        "only an id whose last label came via the mod"
    );
    Ok(())
}
