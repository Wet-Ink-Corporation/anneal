//! `anneal orchestrate fan-out|park|rule`: the deterministic steps of the `/orchestrate` skill.
//!
//! - `fan-out` reads the ticket graph as `dag frontier` does and exits 0 only when its frontier
//!   holds at least [`MIN_FAN_OUT`] independent tickets (ready tickets: no open blocker, so none
//!   waits on another); otherwise 3, with a finding. It writes nothing.
//! - `park` parks a one-way call (Clef D2 said `park`, in effect enforce) in weigh-in with a
//!   default, through the weigh-in bridge (`crate::weighin`), and records `weighin_parked`;
//!   when the bridge cannot (no `weighin`, or it failed) it records `bridge.unavailable` and
//!   exits 4: the orchestrator rules instead.
//! - `rule` records a call the orchestrator ruled on itself as `orchestrate.ruled`: what, why
//!   and the cost if wrong.
//!
//! anneal stores no decision: weigh-in owns parked items and their rulings. See
//! `docs/orchestrate.md`.

mod park;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::dag::{self, Source, ticket::TicketId};
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::render::command_help;
use crate::exit_codes::{self, Code};
use crate::weighin::Runner;
use park::{ParkArgs, RuleArgs};

/// Fewest independent tickets that make fanning out worth its cost.
pub const MIN_FAN_OUT: usize = 3;

/// `anneal orchestrate`.
#[derive(Debug, Args)]
pub struct OrchestrateArgs {
    #[command(subcommand)]
    action: OrchestrateAction,
}

/// The `anneal orchestrate` subcommands.
#[derive(Debug, Subcommand)]
pub enum OrchestrateAction {
    /// Exit 0 only when the frontier has 3 or more independent tickets to fan out over
    #[command(name = "fan-out", after_help = command_help(CommandName::Orchestrate))]
    FanOut(FanOutArgs),
    /// Park a one-way call in weigh-in with a default; exit 4 (rule instead) when weigh-in is
    /// unavailable
    #[command(after_help = command_help(CommandName::Orchestrate))]
    Park(ParkArgs),
    /// Record a call ruled on without weigh-in: what, why and the cost if wrong
    #[command(after_help = command_help(CommandName::Orchestrate))]
    Rule(RuleArgs),
}

/// `anneal orchestrate fan-out`.
#[derive(Debug, Args)]
pub struct FanOutArgs {
    /// Where the tickets are.
    #[command(flatten)]
    source: Source,
}

/// What `anneal orchestrate` reaches outside itself: the working directory, the environment and
/// the `weighin` CLI.
pub(crate) struct Ports<'a, L, R> {
    /// The working directory.
    pub(crate) cwd: &'a Path,
    /// Reads an environment variable.
    pub(crate) lookup: L,
    /// Runs `weighin`.
    pub(crate) runner: &'a R,
}

/// Runs `anneal orchestrate <action>` through `ports`.
///
/// # Errors
/// [`CliError::Usage`] for refused arguments, [`CliError::Dag`] when the tickets cannot be read,
/// the ledger's errors, and [`CliError::Output`].
pub(crate) fn run<O, L, R>(
    args: &OrchestrateArgs,
    ports: &Ports<'_, L, R>,
    out: &mut O,
) -> Result<Code, CliError>
where
    O: Write,
    L: Fn(&str) -> Option<OsString>,
    R: Runner,
{
    match &args.action {
        OrchestrateAction::FanOut(fan_out_args) => fan_out(fan_out_args, ports.cwd, out),
        OrchestrateAction::Park(park_args) => park::park(park_args, ports, out),
        OrchestrateAction::Rule(rule_args) => park::rule(rule_args, ports, out),
    }
}

/// `data` of `orchestrate fan-out`.
#[derive(Debug, Serialize)]
struct FanOutData<'g> {
    tracker: &'static str,
    fan_out: bool,
    independent: usize,
    required: usize,
    tickets: Vec<&'g TicketId>,
}

/// `orchestrate fan-out`: the frontier's size against [`MIN_FAN_OUT`].
fn fan_out<O: Write>(args: &FanOutArgs, cwd: &Path, out: &mut O) -> Result<Code, CliError> {
    let tracker = dag::tracker(&args.source, cwd)?;
    let graph = dag::read_graph(tracker.as_ref())?;
    let verdict = graph.check();
    let tickets: Vec<&TicketId> = if verdict.code().is_success() {
        graph.frontier().map(|ticket| &ticket.id).collect()
    } else {
        Vec::new()
    };
    let go = tickets.len() >= MIN_FAN_OUT;
    let mut findings = dag::check_findings(&verdict);
    if !go {
        findings.push(Finding::new(
            "orchestrate/no-fan-out",
            format!(
                "the frontier has {} independent tickets; fanning out needs {MIN_FAN_OUT}: run \
                 them one at a time",
                tickets.len()
            ),
        ));
    }
    let code = if go {
        exit_codes::ORCHESTRATE_OK
    } else {
        exit_codes::ORCHESTRATE_NO_FAN_OUT
    };
    let data = FanOutData {
        tracker: tracker.name(),
        fan_out: go,
        independent: tickets.len(),
        required: MIN_FAN_OUT,
        tickets,
    };
    Envelope::new(CommandName::Orchestrate, Some("fan-out"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
