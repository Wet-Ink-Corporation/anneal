//! Reading the decision log `anneal decide` writes: every file, oldest first, into the
//! answered decisions, the unanswered counts and the latest label of each id (and whether it
//! came through a mod).
//!
//! Each file is read whole, and must be a regular file (checked by `symlink_metadata` before
//! it is opened) of at most [`MAX_LOG_FILE_BYTES`]: decide's 16 MiB rotation size plus
//! [`HEADROOM_ROWS`] rows of its 8 KiB row cap. `decide` appends without a lock, so concurrent
//! calls can each pass its size check and leave the live file a few rows past 16 MiB; a file
//! larger still was not written by `decide` and is an error. Rows are read tolerantly (unknown
//! fields ignored), as a newer `decide` may add some; a line that is not a decision or label
//! row this build reads, or a probability outside 0..=1, is counted in `skipped_rows`, not
//! dropped silently and not fatal, since one torn line must not stop calibration.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::baseline::BaselineError;
use crate::calibrate::CalibrateError;
use crate::decide::label::Via;
use crate::decide::log::{DecisionLog, MAX_FILE_BYTES};
use crate::decide::point::{Point, Question};

/// Decide's row cap (`decide::log::MAX_ROW_BYTES`) as a `u64`; a test keeps them equal.
pub(crate) const ROW_BYTES: u64 = 8 * 1024;

/// How many rows past the rotation size a live file may hold: concurrent appends that each
/// passed decide's size check before any of them landed.
pub(crate) const HEADROOM_ROWS: u64 = 64;

/// The largest log file read: the rotation size plus the headroom (16.5 MiB).
pub(crate) const MAX_LOG_FILE_BYTES: u64 = MAX_FILE_BYTES + HEADROOM_ROWS * ROW_BYTES;

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
    /// The decision ids whose latest label came through a mod (`via: mod`).
    pub(crate) via_mod: HashSet<String>,
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
    /// Read as text, so a label through something a newer build names still reads; absent on
    /// rows written before `--via` and on rows written without it.
    via: Option<String>,
}

/// Reads every file of `log`, oldest first.
///
/// # Errors
/// [`CalibrateError::Read`] when a file cannot be read or is not a regular file;
/// [`CalibrateError::TooLarge`] when one is over [`MAX_LOG_FILE_BYTES`].
pub(crate) fn read_log(log: &DecisionLog) -> Result<LogRows, CalibrateError> {
    let mut rows = LogRows::default();
    for path in log.files() {
        let Some(bytes) = read_file(&path)? else {
            continue;
        };
        read_into(&bytes, &mut rows);
        rows.files.push(path);
    }
    Ok(rows)
}

/// The whole of the log file at `path`, or `None` when there is none.
fn read_file(path: &Path) -> Result<Option<Vec<u8>>, CalibrateError> {
    let io_error = |source| {
        CalibrateError::Read(BaselineError::Io {
            op: "reading",
            path: path.to_path_buf(),
            source,
        })
    };
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_other) => {
            return Err(CalibrateError::Read(BaselineError::NotAFile {
                path: path.to_path_buf(),
            }));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error(source)),
    }
    let file = match File::open(path) {
        Ok(file) => file,
        // Rotated away since its metadata was read.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error(source)),
    };
    let mut bytes = Vec::new();
    // One byte past the cap, so a file over it is reported, never silently cut.
    file.take(MAX_LOG_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_LOG_FILE_BYTES) {
        return Err(CalibrateError::TooLarge {
            path: path.to_path_buf(),
            cap: MAX_LOG_FILE_BYTES,
        });
    }
    Ok(Some(bytes))
}

/// Adds the rows of one file to `rows`.
fn read_into(bytes: &[u8], rows: &mut LogRows) {
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice(line) {
            Ok(RowDto::Decision(decision)) => add_decision(decision, rows),
            Ok(RowDto::Label(label)) => add_label(label, rows),
            Err(_not_a_row) => rows.skipped_rows = rows.skipped_rows.saturating_add(1),
        }
    }
}

/// Records `label` as its id's latest, with where it came from: a later label replaces an
/// earlier one's outcome and its `via` alike.
fn add_label(label: LabelDto, rows: &mut LogRows) {
    if label.via.as_deref() == Some(Via::Mod.as_str()) {
        // The set and the map each own their copy of the id.
        rows.via_mod.insert(label.id.clone());
    } else {
        rows.via_mod.remove(&label.id);
    }
    rows.labels.insert(label.id, label.outcome);
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
    // `-0.0` passes the range check but sorts apart from `0.0` under `total_cmp`; adding
    // `0.0` turns it into `0.0` and leaves every other value as it is.
    let probability = probability + 0.0;
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
