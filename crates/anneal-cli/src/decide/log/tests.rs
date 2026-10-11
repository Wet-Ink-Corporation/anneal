use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::{DecisionLog, LOG_FILE, LogError, MAX_ROW_BYTES, ROTATED_FILES};
use crate::decide::point::Point;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .expect("the log is readable")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect()
}

#[test]
fn rows_are_appended_one_json_line_each_under_home_decisions() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());

    log.append(&json!({"n": 1}))?;
    log.append(&json!({"n": 2}))?;

    let path = home.path().join("decisions").join(LOG_FILE);
    assert_eq!(log.path(), path);
    assert_eq!(lines(&path), [json!({"n": 1}), json!({"n": 2})]);
    Ok(())
}

#[test]
fn a_row_over_the_cap_is_refused_and_nothing_is_written() {
    let home = tempfile::tempdir().expect("tempdir");
    let log = DecisionLog::in_home(home.path());

    let result = log.append(&json!({"big": "x".repeat(MAX_ROW_BYTES)}));

    assert!(
        matches!(result, Err(LogError::RowTooLarge { .. })),
        "{result:?}"
    );
    assert!(!log.path().exists(), "no file created");
}

#[test]
fn a_full_log_rotates_and_keeps_only_the_newest_old_files() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog {
        dir: home.path().join("decisions"),
        max_file_bytes: 20,
    };

    // Each row is 8 bytes with its newline, so a file holds two rows and the third rotates:
    // 12 rows make 6 files, of which the live one and 4 old ones are kept.
    for n in 0..12 {
        log.append(&json!({"n": n % 10}))?;
    }

    assert_eq!(
        lines(&log.path()).len(),
        2,
        "the live file holds the newest rows"
    );
    for generation in 1..=ROTATED_FILES {
        assert!(
            log.rotated(generation).exists(),
            "generation {generation} kept"
        );
    }
    assert!(
        !log.rotated(ROTATED_FILES + 1).exists(),
        "older ones dropped"
    );
    assert_eq!(
        lines(&log.rotated(1)),
        [json!({"n": 8}), json!({"n": 9})],
        "generation 1 is the previous file"
    );
    Ok(())
}

#[test]
fn something_other_than_a_file_at_the_log_path_is_refused() {
    let home = tempfile::tempdir().expect("tempdir");
    let log = DecisionLog::in_home(home.path());
    fs::create_dir_all(log.path()).expect("a directory where the log goes");

    let result = log.append(&json!({}));

    assert!(
        matches!(result, Err(LogError::NotAFile { .. })),
        "{result:?}"
    );
}

#[test]
fn answered_counts_start_at_zero_and_persist_per_point() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());
    assert_eq!(log.answered(Point::D1)?, 0);

    log.count_answered(Point::D1)?;
    log.count_answered(Point::D1)?;
    log.count_answered(Point::D3)?;

    let reopened = DecisionLog::in_home(home.path());
    assert_eq!(reopened.answered(Point::D1)?, 2);
    assert_eq!(reopened.answered(Point::D3)?, 1);
    assert_eq!(reopened.answered(Point::D2)?, 0);
    Ok(())
}

#[test]
fn a_counts_file_of_another_kind_is_refused() {
    let home = tempfile::tempdir().expect("tempdir");
    let log = DecisionLog::in_home(home.path());
    fs::create_dir_all(&log.dir).expect("mkdir");
    fs::write(
        log.counts_path(),
        r#"{"kind":"other","version":1,"answered":{}}"#,
    )
    .expect("write");

    assert!(matches!(
        log.answered(Point::D1),
        Err(LogError::CountsForeign { .. })
    ));
}

#[test]
fn rotating_a_live_file_another_call_already_rotated_continues() -> TestResult {
    let home = tempfile::tempdir()?;
    let log = DecisionLog::in_home(home.path());
    fs::create_dir_all(&log.dir)?;
    fs::write(log.rotated(1), "{\"n\":1}\n")?;

    log.rotate()?;

    assert!(!log.path().exists(), "no live file appears");
    assert_eq!(lines(&log.rotated(2)), [json!({"n": 1})], "older moved up");
    Ok(())
}

#[test]
fn a_directory_at_the_counts_path_is_refused() {
    let home = tempfile::tempdir().expect("tempdir");
    let log = DecisionLog::in_home(home.path());
    fs::create_dir_all(log.counts_path()).expect("a directory where counts go");

    assert!(matches!(log.answered(Point::D1), Err(LogError::Counts(_))));
}
