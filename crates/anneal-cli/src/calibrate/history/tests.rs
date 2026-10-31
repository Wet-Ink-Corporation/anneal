use super::{DAY_MS, DayScore, HISTORY_DAYS, history};
use crate::calibrate::read::{Answered, LogRows};
use crate::calibrate::report::report;
use crate::decide::point::{Point, Question};
use crate::decide::thresholds::Thresholds;

/// A D1 noul decision at `probability`, decided at `ts_ms`.
fn push(rows: &mut LogRows, id: &str, probability: f64, ts_ms: Option<u64>) {
    rows.answered.push(Answered {
        id: id.to_owned(),
        point: Point::D1,
        question: Question::Noul,
        probability,
        top: None,
        options: vec!["no".to_owned(), "yes".to_owned()],
        asked: false,
    });
    rows.answered_ms.push(ts_ms);
}

/// A D1 decision of `question` at 0.5, decided at `ts_ms`.
fn push_question(rows: &mut LogRows, id: &str, question: Question, ts_ms: Option<u64>) {
    push(rows, id, 0.5, ts_ms);
    if let Some(last) = rows.answered.last_mut() {
        last.question = question;
    }
}

/// D1 noul's `n` in the report of `rows` against the shipped thresholds.
fn d1_noul_n(rows: &LogRows) -> u32 {
    let thresholds = Thresholds::built_in().expect("the shipped thresholds are valid");
    let report = report(&thresholds, rows);
    report
        .points
        .iter()
        .find(|point| point.point == Point::D1)
        .and_then(|point| point.questions.first())
        .expect("D1 reports noul")
        .n
}

fn label(rows: &mut LogRows, id: &str, outcome: &str) {
    rows.labels.insert(id.to_owned(), outcome.to_owned());
}

#[test]
fn decisions_are_counted_and_scored_per_utc_day_of_their_ts_ms() {
    let mut rows = LogRows::default();
    let monday = 20_000 * DAY_MS;
    push(&mut rows, "a", 0.5, Some(monday));
    push(&mut rows, "b", 1.0, Some(monday + DAY_MS - 1));
    push(&mut rows, "c", 0.2, Some(monday + DAY_MS));
    label(&mut rows, "a", "yes");
    label(&mut rows, "b", "maybe");
    label(&mut rows, "c", "no");

    let history = history(&rows);

    assert_eq!(
        history.get(&Point::D1).map(Vec::as_slice),
        Some(
            &[
                DayScore {
                    question: Question::Noul,
                    day_start_ms: monday,
                    n: 2,
                    labelled: 1,
                    brier: Some(0.25),
                },
                DayScore {
                    question: Question::Noul,
                    day_start_ms: monday + DAY_MS,
                    n: 1,
                    labelled: 1,
                    brier: Some(0.040_000_000_000_000_01),
                },
            ][..]
        ),
        "an unusable label counts toward n only"
    );
    assert_eq!(history.get(&Point::D2), None);
}

#[test]
fn only_the_latest_days_are_kept_and_rows_without_ts_ms_are_left_out() {
    let mut rows = LogRows::default();
    push(&mut rows, "undated", 0.5, None);
    let days = u64::try_from(HISTORY_DAYS).expect("a small number") + 5;
    for day in 0..days {
        push(&mut rows, &format!("d{day}"), 0.5, Some(day * DAY_MS));
    }

    let history = history(&rows);

    let kept: Vec<u64> = history
        .get(&Point::D1)
        .expect("D1 has days")
        .iter()
        .map(|day| day.day_start_ms / DAY_MS)
        .collect();
    assert_eq!(kept, (5..days).collect::<Vec<_>>());
}

#[test]
fn each_question_keeps_its_own_latest_days_listed_by_question_then_oldest_day() {
    let mut rows = LogRows::default();
    let days = u64::try_from(HISTORY_DAYS).expect("a small number") + 3;
    // Newest first and choice before noul in the log, so the order comes from the history.
    for day in (0..days).rev() {
        let at = Some(day * DAY_MS);
        push_question(&mut rows, &format!("c{day}"), Question::Choice, at);
        push_question(&mut rows, &format!("n{day}"), Question::Noul, at);
    }

    let history = history(&rows);

    let listed: Vec<(Question, u64)> = history
        .get(&Point::D1)
        .expect("D1 has days")
        .iter()
        .map(|day| (day.question, day.day_start_ms / DAY_MS))
        .collect();
    let latest = || 3..days;
    let expected: Vec<(Question, u64)> = latest()
        .map(|day| (Question::Noul, day))
        .chain(latest().map(|day| (Question::Choice, day)))
        .collect();
    assert_eq!(listed, expected);
}

#[test]
fn a_decision_without_a_timestamp_entry_is_left_out_of_the_history_alone() {
    let mut rows = LogRows::default();
    push(&mut rows, "dated", 0.5, Some(DAY_MS));
    push(&mut rows, "short", 0.5, Some(DAY_MS));
    push(&mut rows, "shorter", 0.5, Some(DAY_MS));
    // As rows built without the timestamps (`report`'s tests) have them: fewer than decisions.
    rows.answered_ms.truncate(1);

    let history = history(&rows);
    let scored = d1_noul_n(&rows);

    let d1 = history.get(&Point::D1).expect("D1 has its dated day");
    assert_eq!(d1.iter().map(|day| day.n).collect::<Vec<_>>(), [1]);
    assert_eq!(scored, 3, "every decision is still scored");
}

#[test]
fn rows_without_any_timestamps_have_no_history_and_keep_their_scores() {
    let mut rows = LogRows::default();
    push(&mut rows, "a", 0.5, None);
    push(&mut rows, "b", 0.5, None);
    rows.answered_ms.clear();

    let history = history(&rows);
    let scored = d1_noul_n(&rows);

    assert!(history.is_empty(), "{history:?}");
    assert_eq!(scored, 2);
}
