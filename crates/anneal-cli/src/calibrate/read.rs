//! Reading the decision log `anneal decide` writes: every file, oldest first, into the
//! answered decisions, the unanswered counts and the latest label of each id.
//!
//! Each file is read whole through [`baseline::read_state_bytes`]: a regular file of at most
//! 16 MiB, the size at which `decide` rotates it. Rows are read tolerantly (unknown fields
//! ignored), as a newer `decide` may add some; a line that is not a decision or label row this
//! build reads, or a probability outside 0..=1, is counted in `skipped_rows`, not dropped
//! silently and not fatal, since one torn line must not stop calibration.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::Deserialize;

use crate::baseline;
use crate::calibrate::CalibrateError;
use crate::decide::log::DecisionLog;
use crate::decide::point::{Point, Question};

/// An answered decision: one with a probability.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Answered {
    /// The decision's id, which labels refer to.
    pub(crate) id: String,
    /// The decision point.
    pub(crate) point: Point,
    /// The question type.
    pub(crate) question: Question,
    /// The probability the thresholds applied to.
    pub(crate) probability: f64,
    /// The top option of a choice.
    pub(crate) top: Option<String>,
    /// The options Clef gave probabilities for, by name.
    pub(crate) options: Vec<String>,
    /// Whether the action handed the decision to a human (`ask`, `ask_*`).
    pub(crate) asked: bool,
}

/// What the log holds, as calibration needs it.
#[derive(Debug, Default)]
pub(crate) struct LogRows {
    /// The files read, oldest first.
    pub(crate) files: Vec<PathBuf>,
    /// Answered decisions, in log order.
    pub(crate) answered: Vec<Answered>,
    /// Decisions without an answer, per point and question.
    pub(crate) unanswered: BTreeMap<(Point, Question), u32>,
    /// The latest outcome label of each decision id.
    pub(crate) labels: HashMap<String, String>,
    /// Lines that are not a row this build reads.
    pub(crate) skipped_rows: u32,
}

/// One line of the log.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RowDto {
    Decision(DecisionDto),
    Label(LabelDto),
}

#[derive(Debug, Deserialize)]
struct DecisionDto {
    id: String,
    point: Point,
    question: Question,
    probability: Option<f64>,
    top: Option<String>,
    probabilities: Option<BTreeMap<String, f64>>,
    action: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LabelDto {
    id: String,
    outcome: String,
}

/// Reads every file of `log`, oldest first.
///
/// # Errors
/// [`CalibrateError::Read`] when a file cannot be read, is not a regular file or is over
/// 16 MiB.
pub(crate) fn read_log(log: &DecisionLog) -> Result<LogRows, CalibrateError> {
    let mut rows = LogRows::default();
    for path in log.files() {
        let Some(bytes) = baseline::read_state_bytes(&path).map_err(CalibrateError::Read)? else {
            continue;
        };
        read_into(&bytes, &mut rows);
        rows.files.push(path);
    }
    Ok(rows)
}

/// Adds the rows of one file to `rows`.
fn read_into(bytes: &[u8], rows: &mut LogRows) {
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice(line) {
            Ok(RowDto::Decision(decision)) => add_decision(decision, rows),
            Ok(RowDto::Label(label)) => {
                rows.labels.insert(label.id, label.outcome);
            }
            Err(_not_a_row) => rows.skipped_rows = rows.skipped_rows.saturating_add(1),
        }
    }
}

fn add_decision(decision: DecisionDto, rows: &mut LogRows) {
    let Some(probability) = decision.probability else {
        let unanswered = rows
            .unanswered
            .entry((decision.point, decision.question))
            .or_insert(0);
        *unanswered = unanswered.saturating_add(1);
        return;
    };
    if !(0.0..=1.0).contains(&probability) {
        rows.skipped_rows = rows.skipped_rows.saturating_add(1);
        return;
    }
    rows.answered.push(Answered {
        id: decision.id,
        point: decision.point,
        question: decision.question,
        probability,
        top: decision.top,
        // A row without `probabilities` names no options, so a choice label cannot match one
        // and counts as unusable; nothing is hidden.
        options: decision
            .probabilities
            .map(|options| options.into_keys().collect())
            .unwrap_or_default(),
        asked: decision.action.as_deref().is_some_and(is_ask),
    });
}

/// An action that hands the decision to a human: `ask`, or a name starting `ask_`.
fn is_ask(action: &str) -> bool {
    action == "ask" || action.starts_with("ask_")
}

#[cfg(test)]
mod tests;
