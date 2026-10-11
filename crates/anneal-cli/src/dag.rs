//! `anneal dag check|frontier`: the ticket graph a tracker holds.
//!
//! - [`ticket`]: [`ticket::Ticket`] and the id order every listing uses.
//! - [`tracker`]: the [`tracker::Tracker`] port and its adapters (`markdown`, `github`).
//! - [`graph`]: the graph, its check (dangling references, then cycles) and its frontier.
//! - [`estimate`]: the per-ticket token estimate.
//!
//! `check` exits 0, 1 on a cycle or 2 on a dangling reference (which wins when both occur);
//! `frontier` runs the same check first and lists no ticket when it fails. A tracker that
//! cannot be read is an error (exit 70). Formats and heuristics: `docs/dag.md`.

pub mod estimate;
pub mod graph;
pub mod ticket;
pub mod tracker;

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::Code;
use crate::exit_codes::render::command_help;
use estimate::{DEFAULT_BUDGET, Estimate};
use graph::{Check, Dangling, Graph, GraphError};
use ticket::{Status, TicketId};
use tracker::github::{GhCli, GitHub, Repo};
use tracker::markdown::Markdown;
use tracker::{Tracker, TrackerError};

/// Arguments of `anneal dag`.
#[derive(Debug, Args)]
pub struct DagArgs {
    /// What to do with the graph.
    #[command(subcommand)]
    pub action: DagAction,
}

/// The `dag` subcommands.
#[derive(Debug, Subcommand)]
pub enum DagAction {
    /// Check the ticket graph: exit 1 on a cycle, 2 on a reference to a missing ticket
    #[command(after_help = command_help(CommandName::Dag))]
    Check(CheckArgs),
    /// List the ready tickets with a token estimate each, flagging any over the budget
    #[command(after_help = command_help(CommandName::Dag))]
    Frontier(FrontierArgs),
}

/// Arguments of `anneal dag check`.
#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Where the tickets are.
    #[command(flatten)]
    pub source: Source,
}

/// Arguments of `anneal dag frontier`.
#[derive(Debug, Args)]
pub struct FrontierArgs {
    /// Where the tickets are.
    #[command(flatten)]
    pub source: Source,
    /// Single-context budget in tokens; a ready ticket estimated above it is flagged
    #[arg(long, value_name = "TOKENS", default_value_t = DEFAULT_BUDGET)]
    pub budget: u64,
}

/// Which tracker to read, and where.
#[derive(Debug, Args)]
pub struct Source {
    /// The tracker holding the tickets
    #[arg(long, value_enum, default_value_t = TrackerKind::Markdown)]
    pub tracker: TrackerKind,
    /// markdown: the tickets directory [default: tickets]
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,
    /// github: the repository, as owner/name
    #[arg(long, value_name = "OWNER/NAME")]
    pub repo: Option<Repo>,
    /// github: the issue whose sub-issues are the tickets [default: every issue]
    #[arg(long, value_name = "NUMBER")]
    pub issue: Option<u64>,
}

/// The trackers `--tracker` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TrackerKind {
    /// `<dir>/*.md` with `blocked_by:` frontmatter.
    Markdown,
    /// GitHub Issues through `gh api`, read-only.
    Github,
}

/// The default `--dir`.
const DEFAULT_DIR: &str = "tickets";

/// Why `anneal dag` could not judge the graph.
#[derive(Debug, thiserror::Error)]
pub enum DagError {
    /// The tracker could not hand over its tickets.
    #[error("reading the {tracker} tickets")]
    Tracker {
        /// The tracker's name.
        tracker: &'static str,
        /// What failed.
        #[source]
        source: TrackerError,
    },
    /// The tickets do not form a graph.
    #[error(transparent)]
    Graph(#[from] GraphError),
}

/// Runs `anneal dag <action>` with relative paths taken from `cwd`, and writes its envelope.
///
/// # Errors
/// [`CliError::Usage`] for flags of the other tracker, [`CliError::Dag`] when the tickets cannot
/// be read, [`CliError::Output`] when `out` cannot be written.
pub fn run<O: Write>(args: &DagArgs, cwd: &Path, out: &mut O) -> Result<Code, CliError> {
    match &args.action {
        DagAction::Check(check_args) => check(tracker(&check_args.source, cwd)?.as_ref(), out),
        DagAction::Frontier(frontier_args) => {
            let tracker = tracker(&frontier_args.source, cwd)?;
            frontier(tracker.as_ref(), cwd, frontier_args.budget, out)
        }
    }
}

/// The tracker `source` selects; flags of the other tracker are a usage error.
fn tracker(source: &Source, cwd: &Path) -> Result<Box<dyn Tracker>, CliError> {
    let usage = |reason: &str| CliError::Usage {
        reason: reason.to_owned(),
    };
    match source.tracker {
        TrackerKind::Markdown => {
            if source.repo.is_some() || source.issue.is_some() {
                return Err(usage("--repo and --issue need --tracker github"));
            }
            let dir = source
                .dir
                .as_deref()
                .unwrap_or_else(|| Path::new(DEFAULT_DIR));
            Ok(Box::new(Markdown::new(cwd.join(dir))))
        }
        TrackerKind::Github => {
            if source.dir.is_some() {
                return Err(usage("--dir needs --tracker markdown"));
            }
            // The tracker stores its repository; the arguments stay borrowed by the caller.
            let repo = source
                .repo
                .clone()
                .ok_or_else(|| usage("--tracker github needs --repo owner/name"))?;
            Ok(Box::new(GitHub::new(GhCli::new(), repo, source.issue)))
        }
    }
}

/// Reads `tracker` into a graph.
fn read_graph(tracker: &dyn Tracker) -> Result<Graph, DagError> {
    let tickets = tracker.read().map_err(|source| DagError::Tracker {
        tracker: tracker.name(),
        source,
    })?;
    Ok(Graph::new(tickets)?)
}

/// The envelope's `data` for `dag check`.
#[derive(Debug, Serialize)]
struct CheckData<'g> {
    tracker: &'static str,
    tickets: usize,
    done: usize,
    edges: usize,
    dangling: Vec<DanglingRef<'g>>,
    cycle: &'g [&'g TicketId],
}

#[derive(Debug, Serialize)]
struct DanglingRef<'g> {
    ticket: &'g TicketId,
    missing: &'g TicketId,
}

/// `dag check`: writes the envelope and returns its code.
fn check<O: Write>(tracker: &dyn Tracker, out: &mut O) -> Result<Code, CliError> {
    let graph = read_graph(tracker)?;
    let verdict = graph.check();
    let data = CheckData {
        tracker: tracker.name(),
        tickets: graph.tickets().len(),
        done: graph
            .tickets()
            .iter()
            .filter(|t| t.status == Status::Done)
            .count(),
        edges: graph.edges(),
        dangling: verdict
            .dangling
            .iter()
            .map(|Dangling { ticket, missing }| DanglingRef { ticket, missing })
            .collect(),
        cycle: verdict.cycle.as_deref().unwrap_or(&[]),
    };
    let findings = check_findings(&verdict);
    Envelope::new(CommandName::Dag, Some("check"), verdict.code(), data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// One finding per dangling reference, then one for the cycle.
fn check_findings(verdict: &Check<'_>) -> Vec<Finding> {
    let dangling = verdict.dangling.iter().map(|Dangling { ticket, missing }| {
        Finding::new(
            format!("dag/dangling:{ticket}->{missing}"),
            format!("{ticket} is blocked by {missing}, which is not a ticket"),
        )
    });
    let cycle = verdict.cycle.iter().map(|cycle| {
        let ids: Vec<&str> = cycle.iter().map(|id| id.as_str()).collect();
        // Back to the first id, so the message shows the loop closing.
        let closed: Vec<&str> = ids.iter().chain(ids.first()).copied().collect();
        Finding::new(
            format!("dag/cycle:{}", ids.join("->")),
            format!(
                "cycle: {} (each is blocked by the next)",
                closed.join(" -> ")
            ),
        )
    });
    dangling.chain(cycle).collect()
}

/// The envelope's `data` for `dag frontier`.
#[derive(Debug, Serialize)]
struct FrontierData<'g> {
    tracker: &'static str,
    budget: u64,
    ready: usize,
    over_budget: usize,
    tickets: Vec<Ready<'g>>,
}

/// One ready ticket.
#[derive(Debug, Serialize)]
struct Ready<'g> {
    id: &'g TicketId,
    title: &'g str,
    blocked_by: &'g [TicketId],
    over_budget: bool,
    estimate: Estimate,
}

/// `dag frontier`: writes the envelope and returns its code. Named files are sized under
/// `root`.
fn frontier<O: Write>(
    tracker: &dyn Tracker,
    root: &Path,
    budget: u64,
    out: &mut O,
) -> Result<Code, CliError> {
    let graph = read_graph(tracker)?;
    let verdict = graph.check();
    let code = verdict.code();
    let tickets: Vec<Ready<'_>> = if code.is_success() {
        graph
            .frontier()
            .map(|ticket| {
                let estimate = estimate::estimate(ticket, root);
                Ready {
                    id: &ticket.id,
                    title: &ticket.title,
                    blocked_by: &ticket.blocked_by,
                    over_budget: estimate.tokens > budget,
                    estimate,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let over: Vec<Finding> = tickets
        .iter()
        .filter(|ready| ready.over_budget)
        .map(|ready| {
            Finding::new(
                format!("dag/over-budget:{}", ready.id),
                format!(
                    "{} is estimated at {} tokens, over the {budget}-token budget",
                    ready.id, ready.estimate.tokens
                ),
            )
        })
        .collect();
    let findings: Vec<Finding> = check_findings(&verdict).into_iter().chain(over).collect();
    let data = FrontierData {
        tracker: tracker.name(),
        budget,
        ready: tickets.len(),
        over_budget: tickets.iter().filter(|ready| ready.over_budget).count(),
        tickets,
    };
    Envelope::new(CommandName::Dag, Some("frontier"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
