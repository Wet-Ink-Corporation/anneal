//! One decision: the point's rule, the mode, the call through the [`Transport`] port, the
//! thresholds applied, and the row appended to the log.
//!
//! The order: the question must be one the point asks (else nothing runs); a point whose mode
//! is `off`, or any point without an account, is [`Outcome::Off`] and logs nothing; otherwise
//! Clef is asked within [`TIMEOUT`] and exactly one row is appended, answered or not. A point
//! configured `enforce` runs as `shadow` until it has [`AUTO_SHADOW_DECISIONS`] answered
//! decisions.
//!
//! The request is Workers AI's chat shape, stable part first: a system message holding the
//! question (`{"point", "question", "subject", "options"}`) and a user message holding the
//! compact state.

mod request;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTimeError};

use serde::Serialize;

use crate::decide::answer::{self, Answer};
use crate::decide::clock::Clock;
use crate::decide::config::{ClefConfig, Mode};
use crate::decide::id::DecisionId;
use crate::decide::log::{DecisionLog, LogError};
use crate::decide::point::{Point, Question};
use crate::decide::state::CompactState;
use crate::decide::thresholds::{PointRules, Rule, Thresholds};
use crate::decide::transport::{ClefRequest, FailureKind, Transport, TransportError};
use crate::error_chain;

/// How long a call to Clef may take, connect to last byte.
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// How many answered decisions a point runs in shadow before `enforce` takes effect.
pub const AUTO_SHADOW_DECISIONS: u64 = 50;

/// The Workers AI REST base; the URL is `<base>/<account>/ai/run/<model>`.
pub const ACCOUNTS_URL: &str = "https://api.cloudflare.com/client/v4/accounts";

/// Most options of a choice (H-13).
pub const MAX_OPTIONS: usize = 32;

/// Longest option.
pub const MAX_OPTION_BYTES: usize = 64;

/// Most characters of a failure's reason kept in a row.
const MAX_REASON_CHARS: usize = 256;

/// The reason when there is an account but no token.
const NO_TOKEN: &str = "no API token configured (cf_api_token)";

/// The question asked.
#[derive(Debug, Clone, Copy)]
pub struct Ask<'a> {
    /// The decision point.
    pub point: Point,
    /// The question type.
    pub question: Question,
    /// The options of a choice (2 to [`MAX_OPTIONS`], distinct); none otherwise.
    pub options: &'a [String],
    /// The compact state.
    pub state: &'a CompactState,
}

/// What a decision runs with.
#[derive(Debug, Clone, Copy)]
pub struct Setup<'s> {
    /// The thresholds table.
    pub thresholds: &'s Thresholds,
    /// Account, token and mode.
    pub config: &'s ClefConfig,
    /// The decision log.
    pub log: &'s DecisionLog,
}

/// Why Clef was not asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OffReason {
    /// The point's mode is `off`.
    ModeOff,
    /// No account is configured.
    NoAccount,
}

/// How a decision ended.
#[derive(Debug)]
pub enum Outcome<'s> {
    /// Clef was not asked and nothing was logged.
    Off(OffReason),
    /// Clef answered; the row is logged.
    Decided(Decision<'s>),
    /// Clef gave no usable answer; the row is logged and the caller applies the fallback.
    NoAnswer(NoAnswer<'s>),
}

/// What every call that reached Clef's turn carries, answered or not.
#[derive(Debug)]
pub struct Stamp {
    /// The row's id.
    pub id: DecisionId,
    /// When the call was made, in Unix milliseconds.
    pub ts_ms: u64,
    /// The mode in effect (`shadow` during the first 50 even when `enforce` is configured).
    pub mode: Mode,
    /// The mode configured for the point.
    pub configured_mode: Mode,
    /// How long the call took.
    pub latency_ms: u64,
}

/// An answered decision.
#[derive(Debug)]
pub struct Decision<'s> {
    /// Id, time, mode and latency.
    pub stamp: Stamp,
    /// The model asked.
    pub model: &'s str,
    /// Clef's answer.
    pub answer: Answer,
    /// The action the thresholds give.
    pub action: &'s str,
    /// The `at_least` of the band that gave it, if one did.
    pub threshold: Option<f64>,
}

/// A call without a usable answer.
#[derive(Debug)]
pub struct NoAnswer<'s> {
    /// Id, time, mode and latency.
    pub stamp: Stamp,
    /// The model asked.
    pub model: &'s str,
    /// Why there is no answer (never holds the token).
    pub failure: TransportError,
    /// What the caller does instead (`ask` for D1, else `fail_open`).
    pub fallback: &'s str,
}

/// Why a decision could not be made or recorded.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// The point does not ask this question type.
    #[error("{point} does not ask {question} questions")]
    Unsupported {
        /// The point.
        point: Point,
        /// The question type.
        question: Question,
    },
    /// The options do not fit the question.
    #[error("--option: {reason}")]
    Options {
        /// What is wrong.
        reason: &'static str,
    },
    /// The thresholds table has no entry for the point.
    #[error("no thresholds for {0}")]
    Unconfigured(Point),
    /// The system clock is before 1970.
    #[error("reading the clock")]
    Clock(#[source] SystemTimeError),
    /// The request could not be encoded.
    #[error("encoding the request")]
    Encode(#[source] serde_json::Error),
    /// The row or the counts could not be written.
    #[error(transparent)]
    Log(#[from] LogError),
}

/// Asks Clef one question and logs the result; see the module documentation for the order.
///
/// # Errors
/// [`AskError::Unsupported`] or [`AskError::Options`] for a question the point does not ask or
/// options that do not fit it (nothing runs); [`AskError::Log`] when the row or counts cannot
/// be written; [`AskError::Clock`] or [`AskError::Encode`] when the call cannot be prepared.
pub fn ask<'s>(
    ask: &Ask<'_>,
    setup: &Setup<'s>,
    transport: &impl Transport,
    clock: &impl Clock,
) -> Result<Outcome<'s>, AskError> {
    let rules = setup
        .thresholds
        .point(ask.point)
        .ok_or(AskError::Unconfigured(ask.point))?;
    let rule = rules.rule(ask.question).ok_or(AskError::Unsupported {
        point: ask.point,
        question: ask.question,
    })?;
    request::check_options(ask.question, ask.options)?;
    let configured_mode = setup.config.mode(ask.point);
    if configured_mode == Mode::Off {
        return Ok(Outcome::Off(OffReason::ModeOff));
    }
    let Some(account) = setup.config.account() else {
        return Ok(Outcome::Off(OffReason::NoAccount));
    };
    let mode = if configured_mode == Mode::Enforce
        && setup.log.answered(ask.point)? < AUTO_SHADOW_DECISIONS
    {
        Mode::Shadow
    } else {
        configured_mode
    };
    let ts_ms = clock.unix_millis().map_err(AskError::Clock)?;
    let call = Call {
        ask,
        rules,
        rule,
        account,
    };
    let (answered, latency_ms) = call.send(setup.config, transport, clock)?;
    let stamp = Stamp {
        id: DecisionId::generate(ts_ms),
        ts_ms,
        mode,
        configured_mode,
        latency_ms,
    };
    match answered {
        Ok(answer) => call.decided(setup.log, stamp, answer),
        Err(failure) => call.unanswered(setup.log, stamp, failure),
    }
}

/// One call's question, its point's rules and the account it goes to.
#[derive(Debug, Clone, Copy)]
struct Call<'a, 's> {
    ask: &'a Ask<'a>,
    rules: &'s PointRules,
    rule: &'s Rule,
    account: &'s str,
}

impl<'s> Call<'_, 's> {
    /// Sends the question (or, without a token, fails as unreachable without sending) and
    /// reads the answer, returning it with the call's latency.
    fn send(
        &self,
        config: &ClefConfig,
        transport: &impl Transport,
        clock: &impl Clock,
    ) -> Result<(Result<Answer, TransportError>, u64), AskError> {
        let Some(token) = config.token() else {
            let failure = TransportError::Unreachable {
                reason: NO_TOKEN.to_owned(),
            };
            return Ok((Err(failure), 0));
        };
        let request = ClefRequest {
            url: format!(
                "{ACCOUNTS_URL}/{}/ai/run/{}",
                self.account,
                self.rules.model()
            ),
            model: self.rules.model(),
            body: request::body(self.ask, self.rule)?,
            token,
            timeout: TIMEOUT,
        };
        let start = clock.monotonic();
        let reply = transport.send(&request);
        let latency_ms = millis(clock.monotonic().saturating_sub(start));
        let answered = reply.and_then(|body| {
            answer::parse(&body, self.ask.question, self.ask.options).map_err(|error| {
                TransportError::BadResponse {
                    reason: error_chain::reason(&error),
                }
            })
        });
        Ok((answered, latency_ms))
    }

    /// The row's fields every call has; the outcome fills the rest.
    fn row<'r>(&'r self, stamp: &'r Stamp) -> Row<'r> {
        Row {
            kind: "decision",
            id: &stamp.id,
            ts_ms: stamp.ts_ms,
            point: self.ask.point,
            question: self.ask.question,
            subject: self.rule.subject(),
            model: self.rules.model(),
            state_sha256: self.ask.state.sha256(),
            probabilities: None,
            top: None,
            probability: None,
            thresholds: self.rule,
            threshold: None,
            action: None,
            mode: stamp.mode,
            configured_mode: stamp.configured_mode,
            latency_ms: stamp.latency_ms,
            outcome: None,
            unreachable: false,
            error: None,
            error_reason: None,
        }
    }

    /// Logs an answered decision and counts it toward the shadow period.
    fn decided(
        self,
        log: &DecisionLog,
        stamp: Stamp,
        answer: Answer,
    ) -> Result<Outcome<'s>, AskError> {
        let applied = self.rule.apply(answer.probability());
        let mut row = self.row(&stamp);
        row.probabilities = Some(answer.probabilities());
        row.top = answer.top();
        row.probability = Some(answer.probability());
        row.threshold = applied.threshold;
        row.action = Some(applied.action);
        log.append(&row)?;
        log.count_answered(self.ask.point)?;
        Ok(Outcome::Decided(Decision {
            stamp,
            model: self.rules.model(),
            answer,
            action: applied.action,
            threshold: applied.threshold,
        }))
    }

    /// Logs a call that got no usable answer.
    fn unanswered(
        self,
        log: &DecisionLog,
        stamp: Stamp,
        failure: TransportError,
    ) -> Result<Outcome<'s>, AskError> {
        let kind = failure.kind();
        let mut row = self.row(&stamp);
        row.unreachable = matches!(kind, FailureKind::Unreachable | FailureKind::TimedOut);
        row.error = Some(kind);
        row.error_reason = Some(cut(&failure.to_string()));
        log.append(&row)?;
        Ok(Outcome::NoAnswer(NoAnswer {
            stamp,
            model: self.rules.model(),
            failure,
            fallback: self.rules.fallback(),
        }))
    }
}

/// One row of the log for a call, answered or not.
#[derive(Debug, Serialize)]
struct Row<'a> {
    kind: &'static str,
    id: &'a DecisionId,
    ts_ms: u64,
    point: Point,
    question: Question,
    subject: &'a str,
    model: &'a str,
    state_sha256: String,
    probabilities: Option<&'a BTreeMap<String, f64>>,
    top: Option<&'a str>,
    probability: Option<f64>,
    thresholds: &'a Rule,
    threshold: Option<f64>,
    action: Option<&'a str>,
    mode: Mode,
    configured_mode: Mode,
    latency_ms: u64,
    outcome: Option<&'a str>,
    unreachable: bool,
    error: Option<FailureKind>,
    error_reason: Option<String>,
}

/// Whole milliseconds of `duration`, saturating.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// `reason` cut to [`MAX_REASON_CHARS`] characters.
fn cut(reason: &str) -> String {
    reason.chars().take(MAX_REASON_CHARS).collect()
}

#[cfg(test)]
mod tests;
