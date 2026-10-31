use super::{DAY_MS, DayScore, HISTORY_DAYS, history};
use crate::calibrate::read::{Answered, LogRows};
use crate::decide::point::{Point, Question};

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
