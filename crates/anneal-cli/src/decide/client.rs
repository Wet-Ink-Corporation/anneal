//! One decision: the point's rule, the mode, the call through the [`Transport`] port, the
//! thresholds applied, and the row appended to the log.
//!
//! The order: the question must be one the point asks (else nothing runs); a malformed
//! configuration is [`Outcome::Misconfigured`] and logs nothing; a point whose mode is `off`,
//! or any point without an account, is [`Outcome::Off`] and logs nothing; otherwise
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
use crate::decide::config::{ClefConfig, ConfigError, Mode};
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

/// Most characters of a failure's reason kept in a row.
const MAX_REASON_CHARS: usize = 256;

/// Most bytes of JSON a row's `input` takes, quotes included; a longer input is cut and ends
/// in `…`. With the largest choice row this keeps a row under the log's 8 KiB cap.
pub const MAX_INPUT_ROW_BYTES: usize = 2 * 1024;

/// The mark a cut input ends with.
const CUT_MARK: char = '…';

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

/// A question, and the input the caller keeps in its row (see [`ask_recording`]).
#[derive(Debug, Clone, Copy)]
pub struct Recorded<'a> {
    /// The question asked.
    pub ask: Ask<'a>,
    /// What the row's `input` field holds, already redacted by the caller; `None` for no field.
    pub input: Option<&'a str>,
}

/// What a decision runs with.
#[derive(Debug, Clone, Copy)]
pub struct Setup<'s> {
    /// The thresholds table.
    pub thresholds: &'s Thresholds,
    /// Account, token and mode, or why they could not be read.
    pub config: Result<&'s ClefConfig, &'s ConfigError>,
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

/// Clef was not asked: why, and the mode configured for the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Off {
    /// Why.
    pub reason: OffReason,
    /// The point's configured mode (`off` for [`OffReason::ModeOff`]).
    pub configured_mode: Mode,
}

/// How a decision ended.
#[derive(Debug)]
pub enum Outcome<'s> {
    /// Clef was not asked and nothing was logged.
    Off(Off),
    /// Clef answered; the row is logged.
    Decided(Decision<'s>),
    /// Clef gave no usable answer; the row is logged and the caller applies the fallback.
    NoAnswer(NoAnswer<'s>),
    /// The configuration is malformed, so Clef was not asked and nothing was logged; the
    /// caller applies the fallback.
    Misconfigured(Misconfigured<'s>),
}

/// A call refused for a malformed account, token or mode.
#[derive(Debug)]
pub struct Misconfigured<'s> {
    /// What is wrong (never holds the token).
    pub problem: &'s ConfigError,
    /// What the caller does instead.
    pub fallback: &'s str,
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
    let recorded = Recorded {
        ask: *ask,
        input: None,
    };
    let asked = ask_recording(&recorded, setup, transport, clock)?;
    match asked.unrecorded {
        Some(failure) => Err(AskError::Log(failure)),
        None => Ok(asked.outcome),
    }
}

/// How a call ended, and why its row or count could not be kept, if it could not.
#[derive(Debug)]
pub struct Asked<'s> {
    /// The outcome, answered or not.
    pub outcome: Outcome<'s>,
    /// The failure to append the row or count the answer, after the call was made.
    pub unrecorded: Option<LogError>,
}

/// As [`ask`], keeping the caller's input in the row and handing back the outcome with any
/// failure to log it.
///
/// A row or count that could not be written is in [`Asked::unrecorded`], so the caller can
/// still act on an answer it has.
///
/// The row's `input` field holds `recorded.input` as given (the caller redacts it), cut to
/// [`MAX_INPUT_ROW_BYTES`] of JSON. A row asked without one has no `input` field.
///
/// # Errors
/// As [`ask`], except that a row or count that cannot be written after the call is in
/// [`Asked::unrecorded`]; [`AskError::Log`] remains for the counts read before the call.
pub fn ask_recording<'s>(
    recorded: &Recorded<'_>,
    setup: &Setup<'s>,
    transport: &impl Transport,
    clock: &impl Clock,
) -> Result<Asked<'s>, AskError> {
    let Recorded { ask, input } = recorded;
    let rules = setup.thresholds.point(ask.point);
    let rule = rules.rule(ask.question).ok_or(AskError::Unsupported {
        point: ask.point,
        question: ask.question,
    })?;
    request::check_options(ask.question, ask.options)?;
    let (config, account, configured_mode) = match configured(setup, ask.point, rules) {
        Ok(configured) => configured,
        Err(not_asked) => {
            return Ok(Asked {
                outcome: not_asked.into(),
                unrecorded: None,
            });
        }
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
        input: *input,
    };
    let (answered, latency_ms) = call.send(config, transport, clock)?;
    let stamp = Stamp {
        id: DecisionId::generate(ts_ms),
        ts_ms,
        mode,
        configured_mode,
        latency_ms,
    };
    Ok(match answered {
        Ok(answer) => call.decided(setup.log, stamp, answer),
        Err(failure) => call.unanswered(setup.log, stamp, failure),
    })
}

/// Why Clef is not asked, before any call: the small outcomes of [`configured`].
enum NotAsked<'s> {
    /// The configuration could not be read.
    Misconfigured(Misconfigured<'s>),
    /// Clef is off for the point.
    Off(Off),
}

impl<'s> From<NotAsked<'s>> for Outcome<'s> {
    fn from(not_asked: NotAsked<'s>) -> Self {
        match not_asked {
            NotAsked::Misconfigured(misconfigured) => Self::Misconfigured(misconfigured),
            NotAsked::Off(off) => Self::Off(off),
        }
    }
}

/// The configuration, account and configured mode to ask `point` with, or why Clef is not
/// asked: misconfigured, its mode `off`, or no account.
fn configured<'s>(
    setup: &Setup<'s>,
    point: Point,
    rules: &'s PointRules,
) -> Result<(&'s ClefConfig, &'s str, Mode), NotAsked<'s>> {
    let config = setup.config.map_err(|problem| {
        NotAsked::Misconfigured(Misconfigured {
            problem,
            fallback: rules.fallback(),
        })
    })?;
    let configured_mode = config.mode(point);
    let off = |reason| {
        NotAsked::Off(Off {
            reason,
            configured_mode,
        })
    };
    if configured_mode == Mode::Off {
        return Err(off(OffReason::ModeOff));
    }
    let account = config.account().ok_or_else(|| off(OffReason::NoAccount))?;
    Ok((config, account, configured_mode))
}

/// One call's question, its point's rules and the account it goes to.
#[derive(Debug, Clone, Copy)]
struct Call<'a, 's> {
    ask: &'a Ask<'a>,
    rules: &'s PointRules,
    rule: &'s Rule,
    account: &'s str,
    input: Option<&'a str>,
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
            input: self.input.map(fit_input),
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
    fn decided(self, log: &DecisionLog, stamp: Stamp, answer: Answer) -> Asked<'s> {
        let applied = self.rule.apply(answer.probability());
        let mut row = self.row(&stamp);
        row.probabilities = Some(answer.probabilities());
        row.top = answer.top();
        row.probability = Some(answer.probability());
        row.threshold = applied.threshold;
        row.action = Some(applied.action);
        // The answer stands whether or not it could be kept; the caller decides what a record
        // failure means.
        let unrecorded = log
            .append(&row)
            .and_then(|()| log.count_answered(self.ask.point))
            .err();
        Asked {
            outcome: Outcome::Decided(Decision {
                stamp,
                model: self.rules.model(),
                answer,
                action: applied.action,
                threshold: applied.threshold,
            }),
            unrecorded,
        }
    }

    /// Logs a call that got no usable answer.
    fn unanswered(self, log: &DecisionLog, stamp: Stamp, failure: TransportError) -> Asked<'s> {
        let kind = failure.kind();
        let mut row = self.row(&stamp);
        row.unreachable = matches!(kind, FailureKind::Unreachable | FailureKind::TimedOut);
        row.error = Some(kind);
        row.error_reason = Some(cut(&failure.to_string()));
        let unrecorded = log.append(&row).err();
        Asked {
            outcome: Outcome::NoAnswer(NoAnswer {
                stamp,
                model: self.rules.model(),
                failure,
                fallback: self.rules.fallback(),
            }),
            unrecorded,
        }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<String>,
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

/// `input` cut, at a char boundary and marked with [`CUT_MARK`], so that its JSON string takes
/// at most [`MAX_INPUT_ROW_BYTES`].
fn fit_input(input: &str) -> String {
    /// The two quotes around the JSON string.
    const QUOTES: usize = 2;
    let room = MAX_INPUT_ROW_BYTES.saturating_sub(QUOTES + CUT_MARK.len_utf8());
    // The JSON bytes of `input[..at]`, and the longest such prefix that leaves room for the mark.
    let mut used = 0_usize;
    let mut cut_at = 0;
    for (at, c) in input.char_indices() {
        if used <= room {
            cut_at = at;
        }
        used = used.saturating_add(json_len(c));
        if used.saturating_add(QUOTES) > MAX_INPUT_ROW_BYTES {
            // `cut_at` is a char boundary of `input`, so the prefix exists.
            let kept = input.get(..cut_at).unwrap_or_default();
            return format!("{kept}{CUT_MARK}");
        }
    }
    input.to_owned()
}

/// How many bytes `c` takes inside a JSON string as `serde_json` writes it.
const fn json_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
        '\0'..='\u{1f}' => 6,
        _ => c.len_utf8(),
    }
}

/// `reason` cut to [`MAX_REASON_CHARS`] characters.
fn cut(reason: &str) -> String {
    reason.chars().take(MAX_REASON_CHARS).collect()
}

#[cfg(test)]
mod tests;
