//! `anneal round start|status --ticket <id>`: the fix-loop round counter, read from the
//! ledger's `round_started` events so it survives compaction and restarts.
//!
//! A ticket's round count is the highest `round` among its `round_started` events (0 with
//! none). Rounds 1 to 3 go back to the same agent; rounds 4 and 5 go to a fresh agent one model
//! tier up (`context: fork` with a named subagent, never `model:` in frontmatter); after round
//! 5 the ticket is parked (exit 3; parking it to weigh-in is #37). `start` records the next
//! round, first writer wins, and records nothing once the ticket is parked; `status` never
//! creates a ledger. See `docs/brief.md`.

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anneal_ledger::event::RoundStarted;
use anneal_ledger::{
    Appended, Condition, EventEnvelope, EventKind, Filter, LedgerEvent, Store, StoreError,
    Timestamp,
};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::brief::ticket::Ticket;
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::existing_ledger::{self, LedgerError};
use crate::exit_codes::{self, Code};
use crate::paths::{self, GIT_TIMEOUT};

/// The last round that runs; after it the ticket is parked.
pub const MAX_ROUNDS: u32 = 5;

/// The last round that goes back to the same agent.
const SAME_AGENT_ROUNDS: u32 = 3;

/// How many times `start` reads and appends again when another writer started a round in
/// between.
const START_ATTEMPTS: usize = 4;

/// How long `status` waits for another process's ledger lock, as `Store::open` does.
const LEDGER_BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// `anneal round`.
#[derive(Debug, Args)]
pub struct RoundArgs {
    #[command(subcommand)]
    action: RoundAction,
}

/// The `anneal round` subcommands.
#[derive(Debug, Subcommand)]
pub enum RoundAction {
    /// Record that the ticket's next fix-loop round started
    Start(StartArgs),
    /// Print the ticket's round count and what its next round does
    Status(StatusArgs),
}

/// `anneal round start`.
#[derive(Debug, Args)]
pub struct StartArgs {
    /// The ticket: an issue number or a dag ticket id
    #[arg(long, value_name = "ID")]
    ticket: String,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
}

/// `anneal round status`.
#[derive(Debug, Args)]
pub struct StatusArgs {
    /// The ticket: an issue number or a dag ticket id
    #[arg(long, value_name = "ID")]
    ticket: String,
}

/// What a round does: who works on it, or that the loop stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Next {
    /// Rounds 1 to 3: the same agent fixes its own work.
    SameAgent,
    /// Rounds 4 and 5: a fresh agent one model tier up.
    FreshAgent,
    /// After round 5: no more rounds; park the ticket.
    Park,
}

impl Next {
    /// What round `round` does (round 0 does not exist, so it parks).
    #[must_use]
    pub const fn for_round(round: u32) -> Self {
        match round {
            0 => Self::Park,
            1..=SAME_AGENT_ROUNDS => Self::SameAgent,
            _ if round <= MAX_ROUNDS => Self::FreshAgent,
            _ => Self::Park,
        }
    }

    /// What the round after `rounds` started rounds does.
    #[must_use]
    pub const fn after(rounds: u32) -> Self {
        match rounds.checked_add(1) {
            Some(round) => Self::for_round(round),
            None => Self::Park,
        }
    }

    /// What the orchestrator does for this round, in one line.
    #[must_use]
    pub const fn guidance(self) -> &'static str {
        match self {
            Self::SameAgent => {
                "send the review package back to the same agent: it fixes its own work"
            }
            Self::FreshAgent => {
                "spawn a fresh agent one model tier up, through `context: fork` with a named \
                 subagent (never `model:` in frontmatter), with the brief and review package"
            }
            Self::Park => {
                "five rounds ran: stop the fix loop and park the ticket to weigh-in (#37)"
            }
        }
    }
}

/// A ticket's rounds as the ledger holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Count {
    /// The highest round started for the ticket, 0 with none.
    rounds: u32,
    /// The position of the last `round_started` of any ticket, 0 with none: a start appends
    /// only if no round started after it.
    last_position: u64,
}

/// The rounds of `ticket` in `store`.
fn count(store: &Store, ticket: &Ticket) -> Result<Count, StoreError> {
    let filter = Filter {
        types: vec![EventKind::RoundStarted],
        tags: Vec::new(),
    };
    let events = store.query(&filter)?;
    let rounds = events
        .iter()
        .filter_map(|recorded| match &recorded.event.event {
            LedgerEvent::RoundStarted(started) if started.ticket() == Some(ticket.as_str()) => {
                Some(started.round())
            }
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let last_position = events.last().map_or(0, |recorded| recorded.position);
    Ok(Count {
        rounds,
        last_position,
    })
}

/// Runs `anneal round <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &RoundArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    match &args.action {
        RoundAction::Start(start) => run_start(start, cwd, lookup, out),
        RoundAction::Status(status) => run_status(status, cwd, lookup, out),
    }
}

/// `data` of `round start` and `round status`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    ticket: &'a str,
    /// Rounds started, including the one `start` recorded.
    rounds: u32,
    /// The round `start` recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    started: Option<u32>,
    /// What the round `start` recorded does, or for `status` the next round.
    next: Next,
    guidance: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<u64>,
    /// The round another `start` recorded first, which this `start` reports instead of
    /// recording one more.
    #[serde(skip_serializing_if = "Option::is_none")]
    joined: Option<u32>,
}

/// `anneal round start`.
fn run_start<O: Write>(
    args: &StartArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let ticket = args.ticket.parse::<Ticket>()?;
    let probe = envelope_for(&ticket, 1, args.session.as_deref())?;
    // The session becomes a ledger tag; check it before a ledger is opened (or created).
    if let Err(error) = probe.tags() {
        return Err(CliError::Usage {
            reason: format!("the --session is recorded as a ledger tag: {error}"),
        });
    }
    let location = paths::ledger_store(None, lookup, cwd)?;
    let store = Store::open(&location.path)?;
    for _ in 0..START_ATTEMPTS {
        let seen = count(&store, &ticket)?;
        let report = match start_once(&store, &ticket, seen, args.session.as_deref())? {
            Attempt::Retry => continue,
            Attempt::Parked { rounds } => Report {
                rounds,
                ..Report::start(Next::Park)
            },
            Attempt::Started { round, position } => Report {
                rounds: round,
                started: Some(round),
                position: Some(position),
                ..Report::start(Next::for_round(round))
            },
            Attempt::Joined { rounds } => Report {
                rounds,
                joined: Some(rounds),
                ..Report::start(Next::for_round(rounds))
            },
        };
        return report.emit(&ticket, out);
    }
    Err(CliError::RoundContended {
        attempts: START_ATTEMPTS,
    })
}

/// What one try of `round start` came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attempt {
    /// Five rounds ran; nothing was appended.
    Parked { rounds: u32 },
    /// This start recorded `round` at `position`.
    Started { round: u32, position: u64 },
    /// Another writer started this ticket's next round first; that round counts for this
    /// start too, and nothing was appended.
    Joined { rounds: u32 },
    /// Another ticket's round was appended after the read; read again.
    Retry,
}

/// Tries once to record the round after `seen` for `ticket`, on the condition that no
/// `round_started` follows the read. On a conflict it reads again: a round of this ticket
/// means another start won (`Joined`), only other tickets' rounds mean `Retry`.
fn start_once(
    store: &Store,
    ticket: &Ticket,
    seen: Count,
    session: Option<&str>,
) -> Result<Attempt, CliError> {
    if Next::after(seen.rounds) == Next::Park {
        return Ok(Attempt::Parked {
            rounds: seen.rounds,
        });
    }
    let round = seen.rounds.saturating_add(1);
    let envelope = envelope_for(ticket, round, session)?;
    let condition = Condition::none_matching(Filter {
        types: vec![EventKind::RoundStarted],
        tags: Vec::new(),
    })
    .after(seen.last_position);
    match store.append_if(&envelope, &condition)? {
        Appended::At(position) => Ok(Attempt::Started { round, position }),
        Appended::Conflict => {
            let now = count(store, ticket)?;
            if now.rounds > seen.rounds {
                Ok(Attempt::Joined { rounds: now.rounds })
            } else {
                Ok(Attempt::Retry)
            }
        }
    }
}

/// The `round_started` envelope for round `round` of `ticket`.
fn envelope_for(
    ticket: &Ticket,
    round: u32,
    session: Option<&str>,
) -> Result<EventEnvelope, CliError> {
    let event = LedgerEvent::RoundStarted(RoundStarted::for_ticket(ticket.as_str(), round));
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.session_id = session.map(str::to_owned);
    Ok(envelope)
}

/// `anneal round status`.
fn run_status<O: Write>(
    args: &StatusArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let ticket = args.ticket.parse::<Ticket>()?;
    let store = existing_ledger::open_existing(cwd, lookup, GIT_TIMEOUT, LEDGER_BUSY_TIMEOUT)
        .map_err(|error| match error {
            LedgerError::CurrentDir(source) => CliError::CurrentDir(source),
            LedgerError::Locate(source) => CliError::Path(source),
            LedgerError::Open(source) => CliError::Ledger(source),
        })?;
    let rounds = match &store {
        Some(store) => count(store, &ticket)?.rounds,
        None => 0,
    };
    let report = Report {
        subcommand: "status",
        rounds,
        started: None,
        next: Next::after(rounds),
        position: None,
        joined: None,
    };
    report.emit(&ticket, out)
}

/// What `start` or `status` reports.
#[derive(Debug, Clone, Copy)]
struct Report {
    subcommand: &'static str,
    /// Rounds started, including one `start` recorded.
    rounds: u32,
    /// The round `start` recorded.
    started: Option<u32>,
    /// What that round does, or else the next round.
    next: Next,
    /// Where `start` recorded its round.
    position: Option<u64>,
    /// The round another start recorded while this one ran, which this start reports.
    joined: Option<u32>,
}

impl Report {
    /// A `start` report whose round does `next`, with nothing recorded yet.
    const fn start(next: Next) -> Self {
        Self {
            subcommand: "start",
            rounds: 0,
            started: None,
            next,
            position: None,
            joined: None,
        }
    }

    /// Writes the envelope for `ticket`: exit 3 when `next` parks, else 0.
    fn emit<O: Write>(self, ticket: &Ticket, out: &mut O) -> Result<Code, CliError> {
        let parked = self.next == Next::Park;
        let code = if parked {
            exit_codes::ROUND_PARK
        } else {
            exit_codes::ROUND_OK
        };
        let findings = if parked {
            vec![Finding::new(
                format!("round/park:{ticket}"),
                format!("ticket {ticket}: {}", self.next.guidance()),
            )]
        } else {
            Vec::new()
        };
        let data = Data {
            ticket: ticket.as_str(),
            rounds: self.rounds,
            started: self.started,
            next: self.next,
            guidance: self.next.guidance(),
            position: self.position,
            joined: self.joined,
        };
        Envelope::new(CommandName::Round, Some(self.subcommand), code, data)
            .with_findings(&findings)
            .emit(out)
            .map_err(CliError::Output)
    }
}

#[cfg(test)]
mod tests;
