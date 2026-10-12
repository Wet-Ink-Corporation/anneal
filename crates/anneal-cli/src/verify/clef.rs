//! Clef D5 on a change: `score defect_risk` on `clef-flash`, through `decide`'s own code
//! (`decide::in_process`, then `client::ask_recording`), so its thresholds, log, modes and shadow
//! count are the command's.
//!
//! Clef gets the number of files and changed lines and the changed paths with their line counts
//! (at most [`MAX_PATHS`], through `safe_prompt`); the row keeps none of it. Only an answer in
//! effect enforce whose row was logged can say skip: shadow, off, a malformed configuration,
//! no answer and a row that could not be logged all leave the review in place (D5 fails open
//! to the review the `verify` skill always ran).

use std::ffi::OsString;

use serde::Serialize;
use serde_json::json;

use crate::decide::DecideEnv;
use crate::decide::client::{self, Ask, Asked, Outcome, Recorded};
use crate::decide::clock::Clock;
use crate::decide::config::Mode;
use crate::decide::in_process::{self, Prepared, Status};
use crate::decide::point::{Point, Question};
use crate::decide::redact::safe_prompt;
use crate::decide::state::CompactState;
use crate::decide::transport::Transport;
use crate::error_chain::reason;
use crate::floor_guard::change::Touched;

/// The action of D5's rule that skips the doubt review (`clef-thresholds.json`).
const SKIP: &str = "skip";

/// How many changed paths Clef is shown (H-13); the state cuts the text to 1,024 characters
/// anyway.
const MAX_PATHS: usize = 64;

/// What became of the question to Clef, as `data.clef.clef`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClefOutcome {
    /// The change is empty: nothing to ask about.
    NotAsked,
    /// No account, or D5's mode is `off`: nothing asked or logged.
    Off,
    /// The account, token or mode is malformed, or the thresholds or anneal home cannot be
    /// read: nothing asked or logged.
    Misconfigured,
    /// Clef answered and the row is logged.
    Decided,
    /// Clef gave no usable answer; the row is logged.
    NoAnswer,
    /// Clef answered but the row or count could not be written: not acted on.
    Unlogged,
    /// The call could not be prepared (the clock, the request, the counts).
    Failed,
}

/// D5's answer about a change, as `data.clef`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ClefReport {
    /// What became of the question.
    pub(crate) clef: ClefOutcome,
    /// The decision's id, to label with `anneal decide label` once review shows defects or none.
    pub(crate) id: Option<String>,
    /// Clef's score, the expected defect level normalised to 0..=1.
    pub(crate) defect_risk: Option<f64>,
    /// The thresholds' action (`doubt_review` or `skip` as shipped); only [`SKIP`] can skip.
    pub(crate) action: Option<String>,
    /// The mode in effect.
    pub(crate) mode: Option<Mode>,
    /// Whether the answer may act: in effect enforce, and logged.
    pub(crate) enforce: bool,
    /// Why there is no usable answer, when there is none.
    pub(crate) reason: Option<String>,
}

impl ClefReport {
    /// A report with outcome `clef` and nothing else.
    const fn bare(clef: ClefOutcome) -> Self {
        Self {
            clef,
            id: None,
            defect_risk: None,
            action: None,
            mode: None,
            enforce: false,
            reason: None,
        }
    }

    /// `clef` with the `reason` why.
    fn failed(clef: ClefOutcome, reason: String) -> Self {
        Self {
            reason: Some(reason),
            ..Self::bare(clef)
        }
    }

    /// Whether D5 says skip and may act on it.
    pub(crate) fn says_skip(&self) -> bool {
        self.enforce && self.action.as_deref() == Some(SKIP)
    }
}

/// Asks D5 about the change `files` (`lines` changed lines in all), with `env`'s configuration,
/// transport and clock.
pub(crate) fn ask<L, T, C>(
    files: &[Touched],
    lines: u64,
    env: &DecideEnv<'_, L, T, C>,
) -> ClefReport
where
    L: Fn(&str) -> Option<OsString>,
    T: Transport,
    C: Clock,
{
    let DecideEnv {
        lookup,
        transport,
        clock,
        ..
    } = env;
    if files.is_empty() {
        return ClefReport::failed(ClefOutcome::NotAsked, "the change is empty".to_owned());
    }
    match in_process::status(Point::D5, lookup) {
        Status::Off => return ClefReport::bare(ClefOutcome::Off),
        Status::Misconfigured(problem) => {
            return ClefReport::failed(ClefOutcome::Misconfigured, reason(&problem));
        }
        Status::On => {}
    }
    let prepared = match Prepared::load(lookup) {
        Ok(prepared) => prepared,
        Err(error) => return ClefReport::failed(ClefOutcome::Misconfigured, reason(&error)),
    };
    let state = match state_for(files, lines) {
        Ok(state) => state,
        Err(why) => return ClefReport::failed(ClefOutcome::Failed, why),
    };
    let recorded = Recorded {
        ask: Ask {
            point: Point::D5,
            question: Question::Score,
            options: &[],
            state: &state,
        },
        // The row keeps no paths: they go to Clef, only the state's hash to the log.
        input: None,
    };
    match client::ask_recording(&recorded, &prepared.setup(), *transport, *clock) {
        Ok(asked) => report(asked),
        Err(error) => ClefReport::failed(ClefOutcome::Failed, reason(&error)),
    }
}

/// The report of a call that was made.
fn report(asked: Asked<'_>) -> ClefReport {
    let Asked {
        outcome,
        unrecorded,
    } = asked;
    match (outcome, unrecorded) {
        (_, Some(failure)) => ClefReport::failed(ClefOutcome::Unlogged, reason(&failure)),
        (Outcome::Decided(decision), None) => ClefReport {
            id: Some(decision.stamp.id.to_string()),
            defect_risk: Some(decision.answer.probability()),
            // The report outlives the thresholds the action is borrowed from.
            action: Some(decision.action.to_owned()),
            mode: Some(decision.stamp.mode),
            enforce: decision.stamp.mode == Mode::Enforce,
            ..ClefReport::bare(ClefOutcome::Decided)
        },
        (Outcome::NoAnswer(missed), None) => ClefReport {
            id: Some(missed.stamp.id.to_string()),
            mode: Some(missed.stamp.mode),
            reason: Some(reason(&missed.failure)),
            ..ClefReport::bare(ClefOutcome::NoAnswer)
        },
        (Outcome::Off(_), None) => ClefReport::bare(ClefOutcome::Off),
        (Outcome::Misconfigured(refused), None) => {
            ClefReport::failed(ClefOutcome::Misconfigured, reason(refused.problem))
        }
    }
}

/// The compact state Clef gets: the counts, and the paths with their lines, redacted.
fn state_for(files: &[Touched], lines: u64) -> Result<CompactState, String> {
    let mut listed: Vec<String> = files
        .iter()
        .take(MAX_PATHS)
        .map(|file| match file.lines {
            Some(lines) => format!("{} ({lines})", file.path),
            None => format!("{} (?)", file.path),
        })
        .collect();
    if files.len() > MAX_PATHS {
        listed.push(format!(
            "and {} more",
            files.len().saturating_sub(MAX_PATHS)
        ));
    }
    let paths = listed.join(", ");
    let state = json!({
        "tool": "verify",
        "input": safe_prompt(&paths),
        "files": files.len(),
        "lines": lines,
    });
    CompactState::parse(&state.to_string()).map_err(|error| reason(&error))
}
