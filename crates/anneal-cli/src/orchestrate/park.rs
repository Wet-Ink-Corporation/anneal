//! `orchestrate park` and `orchestrate rule`: a call parked in weigh-in, or ruled on and
//! logged.
//!
//! Every argument is checked before anything runs or a ledger is opened: the ticket as
//! `anneal round` takes it, each text one non-blank line within its cap, `--resolve-by` a
//! `YYYY-MM-DD` date, `--decision` a decision id and `--session` a ledger tag. A park opens the
//! ledger before it runs `weighin`, so a ledger that cannot be opened stops it before anything
//! is parked.

use std::ffi::OsString;
use std::io::Write;

use anneal_ledger::event::{BridgeUnavailable, OrchestrateRuled, WeighinParked};
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};
use clap::Args;
use serde::Serialize;

use crate::brief::ticket::Ticket;
use crate::command::CommandName;
use crate::decide::id::DecisionId;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::error_chain;
use crate::exit_codes::{self, Code};
use crate::hook::injection_scan::detect;
use crate::paths;
use crate::weighin::{self, ItemId, ParkRequest, Runner};

use super::Ports;

mod parked;

use parked::{Park, Parked};

/// Most characters of a question.
const MAX_QUESTION_CHARS: usize = 1024;

/// Most characters of every other text.
const MAX_TEXT_CHARS: usize = 512;

/// `anneal orchestrate park`.
#[derive(Debug, Args)]
pub struct ParkArgs {
    /// The ticket the call blocks: an issue number or a dag ticket id
    #[arg(long, value_name = "ID")]
    ticket: String,
    /// The question for the human, one line
    #[arg(long = "q", value_name = "QUESTION")]
    question: String,
    /// The option taken if the human does not decide by --resolve-by
    #[arg(long, value_name = "OPTION")]
    default: String,
    /// The fact that would flip the call
    #[arg(long, value_name = "FACT")]
    flip: String,
    /// Where the call arises, as path:line
    #[arg(long, value_name = "PATH:LINE")]
    at: String,
    /// When the default is taken, YYYY-MM-DD
    #[arg(long, value_name = "DATE")]
    resolve_by: String,
    /// What the orchestrator does meanwhile
    #[arg(long, value_name = "TEXT", default_value = "blocked")]
    did: String,
    /// The Clef D2 decision id that said park, from its `data.id`
    #[arg(long, value_name = "ID")]
    decision: Option<String>,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
}

/// `anneal orchestrate rule`.
#[derive(Debug, Args)]
pub struct RuleArgs {
    /// The ticket the call was made in
    #[arg(long, value_name = "ID")]
    ticket: String,
    /// What was ruled
    #[arg(long, value_name = "TEXT")]
    what: String,
    /// Why
    #[arg(long, value_name = "TEXT")]
    why: String,
    /// What undoing it costs if the ruling is wrong
    #[arg(long, value_name = "TEXT")]
    cost: String,
    /// The Clef D2 decision id behind the ruling, from its `data.id`
    #[arg(long, value_name = "ID")]
    decision: Option<String>,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
}

/// `data` of `orchestrate park`.
#[derive(Debug, Serialize)]
struct ParkData<'a> {
    ticket: &'a str,
    parked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<&'a str>,
    /// False when this ticket's park of the item was already in the ledger.
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    position: u64,
}

/// `data` of `orchestrate rule`.
#[derive(Debug, Serialize)]
struct RuleData<'a> {
    ticket: &'a str,
    position: u64,
}

/// Where a park or ruling is recorded: the ledger, the ticket and the session.
struct Record<'a> {
    store: &'a Store,
    ticket: &'a Ticket,
    session: Option<&'a str>,
}

impl Record<'_> {
    /// Appends `event`, written by the CLI for the session.
    fn append(&self, event: LedgerEvent) -> Result<u64, CliError> {
        Ok(self.store.append(&envelope(event, self.session)?)?)
    }
}

/// `orchestrate park`: see the module documentation.
pub(super) fn park<O, L, R>(
    args: &ParkArgs,
    ports: &Ports<'_, L, R>,
    out: &mut O,
) -> Result<Code, CliError>
where
    O: Write,
    L: Fn(&str) -> Option<OsString>,
    R: Runner,
{
    let ticket = args.ticket.parse::<Ticket>()?;
    let request = ParkRequest {
        question: one_line("q", &args.question, MAX_QUESTION_CHARS)?,
        did: one_line("did", &args.did, MAX_TEXT_CHARS)?,
        flip: one_line("flip", &args.flip, MAX_TEXT_CHARS)?,
        at: one_line("at", &args.at, MAX_TEXT_CHARS)?,
        blocks: ticket.as_str(),
        resolve_by: date("resolve-by", &args.resolve_by)?,
        default: one_line("default", &args.default, MAX_TEXT_CHARS)?,
    };
    let decision = decision_id(args.decision.as_deref())?;
    let session = args.session.as_deref();
    check_session(&ticket, session)?;
    let store = Store::open(&paths::ledger_store(None, &ports.lookup, ports.cwd)?.path)?;
    let record = Record {
        store: &store,
        ticket: &ticket,
        session,
    };
    let parked = weighin::locate(&ports.lookup)
        .and_then(|located| weighin::park(ports.runner, &located.path, ports.cwd, &request));
    match parked {
        Ok(item) => record_parked(&record, &item, decision, out),
        Err(error) => record_unavailable(&record, &error_chain::reason(&error), out),
    }
}

/// Appends `bridge.unavailable` with `reason` and writes the exit-4 envelope.
fn record_unavailable<O: Write>(
    record: &Record<'_>,
    reason: &str,
    out: &mut O,
) -> Result<Code, CliError> {
    let ticket = record.ticket;
    let position = record.append(LedgerEvent::BridgeUnavailable(BridgeUnavailable {
        ticket: ticket.as_str().to_owned(),
        reason: reason.to_owned(),
    }))?;
    let findings = [Finding::new(
        format!("orchestrate/bridge-unavailable:{ticket}"),
        format!("weigh-in could not park the call ({reason}): rule on it instead"),
    )];
    let data = ParkData {
        ticket: ticket.as_str(),
        parked: false,
        item_id: None,
        recorded: None,
        reason: Some(reason),
        position,
    };
    let code = exit_codes::ORCHESTRATE_UNAVAILABLE;
    emit(
        &Envelope::new(CommandName::Orchestrate, Some("park"), code, data).with_findings(&findings),
        out,
    )
}

/// Appends `weighin_parked` for `item` and the ticket unless the ledger already has it, and
/// writes the envelope.
fn record_parked<O: Write>(
    record: &Record<'_>,
    item: &ItemId,
    decision: Option<DecisionId>,
    out: &mut O,
) -> Result<Code, CliError> {
    let ticket = record.ticket;
    let parks = parked::parks_of(ticket.as_str()).map_err(|error| CliError::Usage {
        reason: format!("the --ticket is recorded as a ledger tag: {error}"),
    })?;
    let event = LedgerEvent::WeighinParked(WeighinParked {
        item_id: item.as_str().to_owned(),
        ticket: ticket.as_str().to_owned(),
        decision: decision.map(|id| id.as_str().to_owned()),
    });
    let envelope = envelope(event, record.session)?;
    let park = Park {
        parks: &parks,
        item: item.as_str(),
        event: &envelope,
    };
    let (position, recorded) = match park.record(record.store)? {
        Parked::Recorded(position) => (position, true),
        Parked::Already(position) => (position, false),
    };
    let data = ParkData {
        ticket: ticket.as_str(),
        parked: true,
        item_id: Some(item.as_str()),
        recorded: Some(recorded),
        reason: None,
        position,
    };
    let code = exit_codes::ORCHESTRATE_OK;
    emit(
        &Envelope::new(CommandName::Orchestrate, Some("park"), code, data),
        out,
    )
}

/// `orchestrate rule`: appends `orchestrate.ruled`.
pub(super) fn rule<O, L, R>(
    args: &RuleArgs,
    ports: &Ports<'_, L, R>,
    out: &mut O,
) -> Result<Code, CliError>
where
    O: Write,
    L: Fn(&str) -> Option<OsString>,
{
    let ticket = args.ticket.parse::<Ticket>()?;
    let ruled = OrchestrateRuled {
        ticket: ticket.as_str().to_owned(),
        what: one_line("what", &args.what, MAX_TEXT_CHARS)?.to_owned(),
        why: one_line("why", &args.why, MAX_TEXT_CHARS)?.to_owned(),
        cost_if_wrong: one_line("cost", &args.cost, MAX_TEXT_CHARS)?.to_owned(),
        decision: decision_id(args.decision.as_deref())?.map(|id| id.as_str().to_owned()),
    };
    let session = args.session.as_deref();
    check_session(&ticket, session)?;
    let store = Store::open(&paths::ledger_store(None, &ports.lookup, ports.cwd)?.path)?;
    let position = store.append(&envelope(LedgerEvent::OrchestrateRuled(ruled), session)?)?;
    let data = RuleData {
        ticket: ticket.as_str(),
        position,
    };
    let code = exit_codes::ORCHESTRATE_OK;
    emit(
        &Envelope::new(CommandName::Orchestrate, Some("rule"), code, data),
        out,
    )
}

/// Writes `envelope`.
fn emit<O: Write, D: Serialize>(envelope: &Envelope<'_, D>, out: &mut O) -> Result<Code, CliError> {
    envelope.emit(out).map_err(CliError::Output)
}

/// The envelope of `event`, written by the CLI for `session`.
fn envelope(event: LedgerEvent, session: Option<&str>) -> Result<EventEnvelope, CliError> {
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.session_id = session.map(str::to_owned);
    Ok(envelope)
}

/// Refuses a `--session` the ledger cannot tag, before a ledger is opened or `weighin` runs.
fn check_session(ticket: &Ticket, session: Option<&str>) -> Result<(), CliError> {
    let probe = LedgerEvent::OrchestrateRuled(OrchestrateRuled {
        ticket: ticket.as_str().to_owned(),
        what: String::new(),
        why: String::new(),
        cost_if_wrong: String::new(),
        decision: None,
    });
    envelope(probe, session)?
        .tags()
        .map(drop)
        .map_err(|error| CliError::Usage {
            reason: format!("the --session is recorded as a ledger tag: {error}"),
        })
}

/// `value` trimmed, when it is one non-blank line of at most `max` characters with no control,
/// default-ignorable, format, line or paragraph separator character (Cc, and what
/// `injection_scan::detect::is_unprintable` names: a bidi override or a U+2028 would hide or
/// break the line on weigh-in's board); else a usage error naming `--flag`.
fn one_line<'a>(flag: &str, value: &'a str, max: usize) -> Result<&'a str, CliError> {
    let text = value.trim();
    let hidden = |c: char| c.is_control() || detect::is_unprintable(c);
    if text.is_empty() || text.chars().any(hidden) || text.chars().count() > max {
        return Err(CliError::Usage {
            reason: format!("--{flag} must be one non-blank line of at most {max} characters"),
        });
    }
    Ok(text)
}

/// `value` when it is a `YYYY-MM-DD` date that exists (month 01 to 12, day 01 to the month's
/// length, February 29 only in a leap year); else a usage error naming `--flag`.
fn date<'a>(flag: &str, value: &'a str) -> Result<&'a str, CliError> {
    let number = |part: &str, digits: usize| {
        (part.len() == digits && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    let valid = match value.split('-').collect::<Vec<_>>().as_slice() {
        [year, month, day] => match (number(year, 4), number(month, 2), number(day, 2)) {
            (Some(year), Some(month), Some(day)) => {
                days_in_month(year, month).is_some_and(|days| (1..=days).contains(&day))
            }
            _ => false,
        },
        _ => false,
    };
    if valid {
        Ok(value)
    } else {
        Err(CliError::Usage {
            reason: format!("--{flag} must be a date, YYYY-MM-DD"),
        })
    }
}

/// The days of `month` (1 to 12) in `year` of the Gregorian calendar; `None` for no month.
const fn days_in_month(year: u32, month: u32) -> Option<u32> {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if leap => Some(29),
        2 => Some(28),
        _ => None,
    }
}

/// The `--decision` id, when given; a malformed one is a usage error.
fn decision_id(value: Option<&str>) -> Result<Option<DecisionId>, CliError> {
    value
        .map(|id| {
            id.parse::<DecisionId>().map_err(|error| CliError::Usage {
                reason: format!("--decision: {error}"),
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::{date, one_line};
    use crate::error::CliError;

    #[test]
    fn a_date_is_four_two_and_two_digits_in_range() {
        for good in ["2026-10-19", "2026-01-01", "2026-12-31"] {
            assert_eq!(date("resolve-by", good).ok(), Some(good), "{good}");
        }
        for bad in [
            "2026-13-01",
            "2026-00-10",
            "2026-10-32",
            "26-10-19",
            "2026-1-19",
            "+026-10-19",
            "",
        ] {
            assert!(
                matches!(date("resolve-by", bad), Err(CliError::Usage { reason })
                    if reason == "--resolve-by must be a date, YYYY-MM-DD"),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_text_is_trimmed_and_capped_in_characters() {
        let refused = |value: &str, max| {
            matches!(one_line("q", value, max), Err(CliError::Usage { reason })
                if reason == format!("--q must be one non-blank line of at most {max} characters"))
        };

        assert_eq!(one_line("q", "  Shared?  ", 8).ok(), Some("Shared?"));
        assert_eq!(one_line("q", &"é".repeat(8), 8).ok(), Some("éééééééé"));
        assert!(refused(&"é".repeat(9), 8), "nine characters");
        assert!(refused("tab\there", 64), "a control character");
    }

    #[test]
    fn a_day_past_the_end_of_its_month_is_refused_with_leap_years_counted() {
        for good in [
            "2024-02-29",
            "2000-02-29",
            "2026-02-28",
            "2026-04-30",
            "2026-01-31",
        ] {
            assert_eq!(date("resolve-by", good).ok(), Some(good), "{good}");
        }
        for bad in [
            "2026-02-29",
            "2100-02-29",
            "2026-02-31",
            "2026-04-31",
            "2026-06-31",
            "2026-09-31",
            "2026-11-31",
        ] {
            assert!(
                matches!(date("resolve-by", bad), Err(CliError::Usage { reason })
                    if reason == "--resolve-by must be a date, YYYY-MM-DD"),
                "{bad}"
            );
        }
    }

    #[test]
    fn invisible_format_and_separator_characters_are_refused_like_control_characters() {
        for character in [
            '\u{2028}', '\u{2029}', '\u{202E}', '\u{200B}', '\u{FEFF}', '\u{00AD}',
        ] {
            let text = format!("Shared{character}cache?");

            let result = one_line("q", &text, 64);

            assert!(
                matches!(&result, Err(CliError::Usage { reason })
                    if reason == "--q must be one non-blank line of at most 64 characters"),
                "U+{:04X}: {result:?}",
                u32::from(character)
            );
        }
    }
}
