//! Clef D1 in the guard: `noul destructive_or_irreversible` for a Bash command the denylist
//! left undecided, asked in process through `decide` (`decide::in_process`, then
//! `client::ask_recording`), so it shares the command's thresholds, modes, shadow count and log.
//!
//! The command is redacted first (`redact`): the state Clef gets is
//! `{"tool": "Bash", "input": {"command": <redacted>}}` and the row keeps the same command as
//! `input`. What the answer does depends on the mode in effect for D1:
//!
//! - off (no account, or mode `off`), or a malformed configuration: nothing is asked, and the
//!   guard answers as it did before D1;
//! - shadow (also `enforce` during its first 50 answered decisions): Clef is asked and the
//!   would-be action logged, and the verdict is unchanged whatever Clef says or fails to say;
//! - enforce: the band's action decides: `deny` denies (one `ERROR` line, `gate_blocked` with
//!   the point `guard.destructive`), `ask` asks the human, anything else (`defer`) leaves the
//!   call to Claude Code's normal permission flow. With no answer (unreachable, timed out, a bad
//!   response) the point's fallback is used when it is `ask`; any other fallback allows, so a
//!   Clef failure is never a deny.
//!
//! In enforce mode a failure never allows: an answer whose row or count could not be written
//! keeps a `deny` or `ask` (the reason notes it) and asks instead of deferring, and D1 failing
//! to run at all (the counts, log, thresholds file or anneal home) asks when D1 is configured
//! `enforce`. Outside enforce mode a failure leaves the verdict alone.
//!
//! The call takes at most `client::TIMEOUT` (2 s) of the hook's 5 s.

use std::ffi::OsString;

use serde_json::json;

use super::redact::redact;
use super::{Decision, Denial, Rule};
use crate::decide::client::{self, Ask, AskError, Decision as Answered, Outcome, Recorded};
use crate::decide::clock::Clock;
use crate::decide::config::{ClefConfig, Mode};
use crate::decide::in_process::{PrepareError, Prepared};
use crate::decide::point::{Point, Question};
use crate::decide::state::{CompactState, StateError};
use crate::decide::transport::Transport;
use crate::error_chain::reason;

/// The thresholds' action and fallback that ask the human.
const ASK: &str = "ask";

/// The thresholds' action that denies.
const DENY: &str = "deny";

/// What D1 reaches outside the process through: Workers AI and the clocks.
#[derive(Debug)]
pub(crate) struct Ports<'a, T, C> {
    /// The port to Workers AI.
    pub(crate) transport: &'a T,
    /// The wall and monotonic clocks.
    pub(crate) clock: &'a C,
}

/// Why D1 could not be asked or recorded.
#[derive(Debug, thiserror::Error)]
enum D1Error {
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Ask(#[from] AskError),
}

/// The guard's decision for `command`, which the denylist did not deny, given D1's answer.
pub(crate) fn judge<T: Transport, C: Clock>(
    command: &str,
    lookup: &impl Fn(&str) -> Option<OsString>,
    ports: &Ports<'_, T, C>,
) -> Decision {
    match ask(&redact(command), lookup, ports) {
        Ok(decision) => decision,
        // D1 could not run (the anneal home, the plugin's thresholds file, the counts, the log
        // or the clock failed). Configured to enforce, it never allows on a failure: the human
        // confirms, as for an unreachable Clef. Otherwise the denylist's verdict stands, as
        // shadow mode requires; an allow has no output to carry the failure, and `anneal
        // decide` with the same environment reports it.
        Err(error) if configured_to_enforce(lookup) => Decision::Ask(format!(
            "anneal guard destructive: Clef D1 could not run ({}); confirm before it runs",
            reason(&error)
        )),
        Err(D1Error::Prepare(_) | D1Error::State(_) | D1Error::Ask(_)) => Decision::Allow,
    }
}

/// Whether D1 is configured `enforce` with an account, whatever its shadow count says.
fn configured_to_enforce(lookup: &impl Fn(&str) -> Option<OsString>) -> bool {
    ClefConfig::from_env(lookup)
        .is_ok_and(|config| config.account().is_some() && config.mode(Point::D1) == Mode::Enforce)
}

/// Asks D1 about the redacted `command`, logging the row, and maps the outcome.
fn ask<T: Transport, C: Clock>(
    command: &str,
    lookup: &impl Fn(&str) -> Option<OsString>,
    ports: &Ports<'_, T, C>,
) -> Result<Decision, D1Error> {
    let prepared = Prepared::load(lookup)?;
    let state = json!({"tool": "Bash", "input": {"command": command}}).to_string();
    let state = CompactState::parse(&state)?;
    let question = Recorded {
        ask: Ask {
            point: Point::D1,
            question: Question::Noul,
            options: &[],
            state: &state,
        },
        input: Some(command),
    };
    let asked = client::ask_recording(&question, &prepared.setup(), ports.transport, ports.clock)?;
    let unlogged = asked
        .unrecorded
        .as_ref()
        .map(|failure| format!(" (the decision was not logged: {})", reason(failure)));
    Ok(decision_for(&asked.outcome, unlogged.as_deref()))
}

/// The guard's decision for D1's outcome; see the module documentation. `unlogged` notes a row
/// or count that could not be written: in enforce mode it never lets the call through
/// unchecked.
fn decision_for(outcome: &Outcome<'_>, unlogged: Option<&str>) -> Decision {
    match outcome {
        Outcome::Decided(answered) if answered.stamp.mode == Mode::Enforce => {
            enforced(answered, unlogged)
        }
        Outcome::NoAnswer(none)
            if none.stamp.mode == Mode::Enforce && (none.fallback == ASK || unlogged.is_some()) =>
        {
            Decision::Ask(format!(
                "anneal guard destructive: Clef D1 could not judge this command ({}){}; \
                 confirm before it runs",
                none.failure,
                unlogged.unwrap_or_default()
            ))
        }
        Outcome::Off(_)
        | Outcome::Misconfigured(_)
        | Outcome::Decided(_)
        | Outcome::NoAnswer(_) => Decision::Allow,
    }
}

/// The decision for an answer in enforce mode: the band's action, except that an answer that
/// could not be logged asks rather than allows.
fn enforced(answered: &Answered<'_>, unlogged: Option<&str>) -> Decision {
    let p = answered.answer.probability();
    let action = answered.action;
    let band = answered
        .threshold
        .map_or_else(String::new, |from| format!(", {action} from {from:.2}"));
    let note = unlogged.unwrap_or_default();
    match (action, unlogged) {
        (DENY, _) => Decision::Deny(Denial::new(
            Rule::Destructive,
            format!(
                "Clef D1 rates this command destructive or irreversible (p {p:.2}{band}){note}"
            ),
        )),
        (ASK, _) => Decision::Ask(format!(
            "anneal guard destructive: Clef D1 rates this command possibly destructive or \
             irreversible (p {p:.2}{band}){note}; confirm before it runs"
        )),
        (_, Some(note)) => Decision::Ask(format!(
            "anneal guard destructive: Clef D1 answered {action} (p {p:.2}){note}; confirm \
             before it runs"
        )),
        (_, None) => Decision::Allow,
    }
}

#[cfg(test)]
mod tests;
