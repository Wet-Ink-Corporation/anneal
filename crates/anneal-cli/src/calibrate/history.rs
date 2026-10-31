//! Each question's answered decisions per UTC day, from the log alone, so a reader can draw a
//! trend: the day of a decision is the UTC day of its `ts_ms`, and its label is the latest
//! one, as the report joins it. Only the latest [`HISTORY_DAYS`] days with decisions are kept
//! per question; a decision without a `ts_ms` is left out.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::calibrate::read::LogRows;
use crate::calibrate::report::outcome;
use crate::calibrate::score::{self, Sample};
use crate::decide::point::{Point, Question};

/// How many days with decisions each question's history keeps, the latest.
const HISTORY_DAYS: usize = 30;

/// Milliseconds in a day; a UTC day starts at a multiple of it.
const DAY_MS: u64 = 86_400_000;

/// One question's answered decisions on one UTC day.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct DayScore {
    pub(crate) question: Question,
    /// The day's start, in Unix milliseconds (UTC).
    pub(crate) day_start_ms: u64,
    /// Answered decisions that day.
    pub(crate) n: u32,
    /// Of `n`, those with a usable label.
    pub(crate) labelled: u32,
    /// The Brier score of the labelled ones; `None` with none.
    pub(crate) brier: Option<f64>,
}

/// One question's decisions on one day while they are counted.
#[derive(Debug, Default)]
struct Tally {
    n: u32,
    samples: Vec<Sample>,
}

/// Each point's days, by question then oldest first; a point with no dated decision is absent.
pub(crate) fn history(rows: &LogRows) -> BTreeMap<Point, Vec<DayScore>> {
    let mut tallies: BTreeMap<(Point, Question, u64), Tally> = BTreeMap::new();
    // `answered_ms` holds one entry per answered decision when the reader filled both. An
    // entry that is missing (rows built without timestamps) means no timestamp: that decision
    // is left out of the history alone, and no other is cut off.
    for (index, decision) in rows.answered.iter().enumerate() {
        let Some(decided_ms) = rows.answered_ms.get(index).copied().flatten() else {
            continue;
        };
        let day_start_ms = decided_ms - decided_ms % DAY_MS;
        let tally = tallies
            .entry((decision.point, decision.question, day_start_ms))
            .or_default();
        tally.n = tally.n.saturating_add(1);
        if let Some(positive) = rows
            .labels
            .get(&decision.id)
            .and_then(|label| outcome(decision, label))
        {
            tally
                .samples
                .push(Sample::new(decision.probability, positive));
        }
    }
    let mut by_question: BTreeMap<(Point, Question), Vec<DayScore>> = BTreeMap::new();
    for ((point, question, day_start_ms), tally) in tallies {
        by_question
            .entry((point, question))
            .or_default()
            .push(DayScore {
                question,
                day_start_ms,
                n: tally.n,
                labelled: score::count(tally.samples.len()),
                brier: score::brier(&tally.samples),
            });
    }
    let mut history: BTreeMap<Point, Vec<DayScore>> = BTreeMap::new();
    for ((point, _question), days) in by_question {
        let older = days.len().saturating_sub(HISTORY_DAYS);
        history
            .entry(point)
            .or_default()
            .extend(days.into_iter().skip(older));
    }
    history
}

#[cfg(test)]
mod tests;
