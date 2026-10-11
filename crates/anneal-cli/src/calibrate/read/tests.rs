use std::fs;

use serde_json::{Value, json};

use super::{Answered, read_log};
use crate::baseline::BaselineError;
use crate::calibrate::CalibrateError;
use crate::decide::log::DecisionLog;
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
