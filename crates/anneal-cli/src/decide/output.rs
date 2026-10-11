//! The envelope `anneal decide` writes for each outcome.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::command::CommandName;
use crate::decide::client::{Ask, Decision, NoAnswer, OffReason, Outcome};
use crate::decide::config::Mode;
use crate::decide::id::DecisionId;
use crate::decide::point::{Point, Question};
use crate::decide::transport::FailureKind;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::error_chain;
use crate::exit_codes::{self, Code};

/// `data.error` of a call refused for a malformed configuration, beside the transport's
/// `unreachable`, `timed_out` and `bad_response`.
const MISCONFIGURED: &str = "misconfigured";

/// `data` of an answered decision.
#[derive(Debug, Serialize)]
struct DecidedData<'a> {
    id: &'a DecisionId,
    point: Point,
    question: Question,
    model: &'a str,
    mode: Mode,
    configured_mode: Mode,
    enforce: bool,
    action: &'a str,
    threshold: Option<f64>,
    probability: f64,
    top: Option<&'a str>,
    probabilities: &'a BTreeMap<String, f64>,
    latency_ms: u64,
    log: &'a Path,
}

/// `data` of a call without an answer.
#[derive(Debug, Serialize)]
struct NoAnswerData<'a> {
    id: &'a DecisionId,
    point: Point,
    question: Question,
    model: &'a str,
    mode: Mode,
    error: FailureKind,
    unreachable: bool,
    fallback: &'a str,
    latency_ms: u64,
    log: &'a Path,
}

/// `data` of a call refused for a malformed configuration.
#[derive(Debug, Serialize)]
struct MisconfiguredData<'a> {
    point: Point,
    question: Question,
    error: &'static str,
    unreachable: bool,
    fallback: &'a str,
    reason: &'a str,
}

/// `data` when Clef is off.
#[derive(Debug, Serialize)]
struct OffData {
    point: Point,
    question: Question,
    mode: Mode,
    configured_mode: Mode,
    reason: OffReason,
}

/// Writes the envelope for `outcome` (the log at `log`) and returns its exit code: 0
/// decided, 3 off, 5 no answer (with one `decide/no-answer:<point>` finding).
pub(super) fn write_outcome<O: Write>(
    ask: &Ask<'_>,
    outcome: &Outcome<'_>,
    log: &Path,
    out: &mut O,
) -> Result<Code, CliError> {
    let emitted = match outcome {
        Outcome::Decided(decision) => decided(ask, decision, log).emit(out),
        Outcome::NoAnswer(no_answer) => {
            let finding = Finding::new(
                format!("decide/no-answer:{}", ask.point),
                no_answer.failure.to_string(),
            );
            let findings = [finding];
            unanswered(ask, no_answer, log)
                .with_findings(&findings)
                .emit(out)
        }
        Outcome::Misconfigured(misconfigured) => {
            let reason = error_chain::reason(misconfigured.problem);
            let findings = [Finding::new(
                format!("decide/no-answer:{}", ask.point),
                reason.as_str(),
            )];
            let data = MisconfiguredData {
                point: ask.point,
                question: ask.question,
                error: MISCONFIGURED,
                unreachable: false,
                fallback: misconfigured.fallback,
                reason: &reason,
            };
            Envelope::new(
                CommandName::Decide,
                None,
                exit_codes::DECIDE_CLEF_UNREACHABLE,
                data,
            )
            .with_findings(&findings)
            .emit(out)
        }
        Outcome::Off(off) => {
            let data = OffData {
                point: ask.point,
                question: ask.question,
                mode: Mode::Off,
                configured_mode: off.configured_mode,
                reason: off.reason,
            };
            Envelope::new(CommandName::Decide, None, exit_codes::DECIDE_OFF, data).emit(out)
        }
    };
    emitted.map_err(CliError::Output)
}

fn decided<'a>(
    ask: &Ask<'_>,
    decision: &'a Decision<'_>,
    log: &'a Path,
) -> Envelope<'a, DecidedData<'a>> {
    let data = DecidedData {
        id: &decision.stamp.id,
        point: ask.point,
        question: ask.question,
        model: decision.model,
        mode: decision.stamp.mode,
        configured_mode: decision.stamp.configured_mode,
        enforce: decision.stamp.mode == Mode::Enforce,
        action: decision.action,
        threshold: decision.threshold,
        probability: decision.answer.probability(),
        top: decision.answer.top(),
        probabilities: decision.answer.probabilities(),
        latency_ms: decision.stamp.latency_ms,
        log,
    };
    Envelope::new(CommandName::Decide, None, exit_codes::DECIDE_DECIDED, data)
}

fn unanswered<'a>(
    ask: &Ask<'_>,
    no_answer: &'a NoAnswer<'_>,
    log: &'a Path,
) -> Envelope<'a, NoAnswerData<'a>> {
    let error = no_answer.failure.kind();
    let data = NoAnswerData {
        id: &no_answer.stamp.id,
        point: ask.point,
        question: ask.question,
        model: no_answer.model,
        mode: no_answer.stamp.mode,
        error,
        unreachable: matches!(error, FailureKind::Unreachable | FailureKind::TimedOut),
        fallback: no_answer.fallback,
        latency_ms: no_answer.stamp.latency_ms,
        log,
    };
    Envelope::new(
        CommandName::Decide,
        None,
        exit_codes::DECIDE_CLEF_UNREACHABLE,
        data,
    )
}
