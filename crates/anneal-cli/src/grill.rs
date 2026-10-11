//! `anneal grill frontier|confirm`: the design tree behind the `grill` skill.
//!
//! - [`tree`]: the design tree file, its checks, its frontier and its confidence summary.
//!
//! `frontier` lists the open questions whose dependencies are all answered. `confirm` records
//! the human's explicit confirmation as a `grill_confirmed` ledger event carrying the tree's
//! hash and the human's words: it refuses a blank `--quote` (a usage error), an invalid tree
//! (exit 1) and a tree with an open question (exit 3), and appends nothing in those cases. See
//! `docs/grill.md`.

pub mod tree;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anneal_ledger::event::GrillConfirmed;
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};
use clap::{Args, Subcommand};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths::{self, StoreSource};
use tree::{
    ConfidenceSummary, DesignTree, MAX_TREE_BYTES, Node, RESTATE_CONFIDENCE,
    RESTATE_TURNS_WITHOUT_GAIN, Status, TreeProblem,
};

/// `anneal grill`.
#[derive(Debug, Args)]
pub struct GrillArgs {
    #[command(subcommand)]
    action: GrillAction,
}

/// The `anneal grill` subcommands.
#[derive(Debug, Subcommand)]
pub enum GrillAction {
    /// List the open questions whose dependencies are all answered, and the restate gate
    Frontier(FrontierArgs),
    /// Record the human's explicit confirmation of the design tree in the ledger
    Confirm(ConfirmArgs),
}

/// `anneal grill frontier`.
#[derive(Debug, Args)]
pub struct FrontierArgs {
    /// The design tree file
    #[arg(long, value_name = "FILE")]
    tree: PathBuf,
}

/// `anneal grill confirm`.
#[derive(Debug, Args)]
pub struct ConfirmArgs {
    /// The design tree file
    #[arg(long, value_name = "FILE")]
    tree: PathBuf,
    /// Required: the human's confirmation, word for word (never inferred). Checked by hand, not
    /// by clap, so a missing or blank quote gets the same one-line reason
    #[arg(long, value_name = "WORDS")]
    quote: Option<String>,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
    /// Use this ledger file instead of the resolved one
    #[arg(long, value_name = "FILE")]
    store: Option<PathBuf>,
}

/// Why `anneal grill` could not read its design tree.
#[derive(Debug, thiserror::Error)]
pub enum GrillError {
    /// The file does not exist.
    #[error("design tree {} does not exist", path.display())]
    Missing {
        /// The path as given.
        path: PathBuf,
    },
    /// The file could not be read.
    #[error("reading design tree {}", path.display())]
    Read {
        /// The path as given.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

impl GrillError {
    /// The documented exit code: a missing file is a usage error (2), a read failure 70.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Missing { .. } => exit_codes::USAGE,
            Self::Read { .. } => exit_codes::INTERNAL,
        }
    }
}

/// Runs `anneal grill <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &GrillArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    match &args.action {
        GrillAction::Frontier(frontier_args) => frontier(frontier_args, cwd, out),
        GrillAction::Confirm(confirm_args) => confirm(confirm_args, cwd, lookup, out),
    }
}

/// The per-turn confidence with the gate's thresholds, as `data.confidence`.
#[derive(Debug, Serialize)]
struct Confidence {
    #[serde(flatten)]
    summary: ConfidenceSummary,
    restate_at: f64,
    restate_after_turns_without_gain: usize,
}

#[derive(Debug, Serialize)]
struct FrontierData<'a> {
    tree: &'a Path,
    hash: &'a str,
    goal: &'a str,
    nodes: usize,
    open: usize,
    answered: usize,
    deferred: usize,
    frontier: Vec<&'a Node>,
    ask: Vec<&'a str>,
    look_up: Vec<&'a str>,
    confidence: Confidence,
}

fn frontier<O: Write>(args: &FrontierArgs, cwd: &Path, out: &mut O) -> Result<Code, CliError> {
    const SUBCOMMAND: &str = "frontier";
    let bytes = read_tree(cwd, &args.tree)?;
    let tree = match DesignTree::parse(&bytes) {
        Ok(tree) => tree,
        Err(problems) => return invalid(SUBCOMMAND, &args.tree, &problems, out),
    };
    let hash = sha256(&bytes);
    let frontier: Vec<&Node> = tree.frontier().collect();
    let ids = |asked: bool| -> Vec<&str> {
        frontier
            .iter()
            .filter(|node| node.is_asked() == asked)
            .map(|node| node.id.as_str())
            .collect()
    };
    let data = FrontierData {
        tree: &args.tree,
        hash: &hash,
        goal: tree.goal(),
        nodes: tree.nodes().len(),
        open: tree.with_status(Status::Open).count(),
        answered: tree.with_status(Status::Answered).count(),
        deferred: tree.with_status(Status::Deferred).count(),
        ask: ids(true),
        look_up: ids(false),
        frontier,
        confidence: Confidence {
            summary: tree.confidence(),
            restate_at: RESTATE_CONFIDENCE,
            restate_after_turns_without_gain: RESTATE_TURNS_WITHOUT_GAIN,
        },
    };
    emit(
        &Envelope::new(CommandName::Grill, Some(SUBCOMMAND), exit_codes::OK, data),
        out,
    )
}

#[derive(Debug, Serialize)]
struct Confirmed<'a> {
    tree: &'a Path,
    store: &'a Path,
    source: StoreSource,
    position: u64,
    event: &'a EventEnvelope,
}

fn confirm<O: Write>(
    args: &ConfirmArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    const SUBCOMMAND: &str = "confirm";
    let Some(quote) = args
        .quote
        .as_deref()
        .filter(|quote| !quote.trim().is_empty())
    else {
        return Err(CliError::Usage {
            reason: "--quote is required: the human's confirmation, word for word".to_owned(),
        });
    };
    let Some(plan) = args.tree.to_str() else {
        return Err(CliError::Usage {
            reason: "the --tree path must be UTF-8: it is recorded as the plan id".to_owned(),
        });
    };
    let bytes = read_tree(cwd, &args.tree)?;
    let tree = match DesignTree::parse(&bytes) {
        Ok(tree) => tree,
        Err(problems) => return invalid(SUBCOMMAND, &args.tree, &problems, out),
    };
    if tree.with_status(Status::Open).next().is_some() {
        return still_open(&args.tree, &tree, out);
    }
    let event = LedgerEvent::GrillConfirmed(GrillConfirmed {
        // The event owns what it records.
        plan: plan.to_owned(),
        hash: Some(sha256(&bytes)),
        quote: Some(quote.to_owned()),
    });
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.session_id.clone_from(&args.session);
    let location = paths::ledger_store(args.store.as_deref(), lookup, cwd)?;
    let position = Store::open(&location.path)?.append(&envelope)?;
    let data = Confirmed {
        tree: &args.tree,
        store: &location.path,
        source: location.source,
        position,
        event: &envelope,
    };
    emit(
        &Envelope::new(CommandName::Grill, Some(SUBCOMMAND), exit_codes::OK, data),
        out,
    )
}

/// Reads the design tree at `path` (relative to `cwd`), at most one byte past
/// [`MAX_TREE_BYTES`] so an oversized file is reported without being read whole.
fn read_tree(cwd: &Path, path: &Path) -> Result<Vec<u8>, GrillError> {
    let failed = |source: io::Error| {
        // The error owns the path it names.
        let path = path.to_path_buf();
        if source.kind() == io::ErrorKind::NotFound {
            GrillError::Missing { path }
        } else {
            GrillError::Read { path, source }
        }
    };
    let file = std::fs::File::open(cwd.join(path)).map_err(failed)?;
    let limit = u64::try_from(MAX_TREE_BYTES).map_or(u64::MAX, |max| max.saturating_add(1));
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).map_err(failed)?;
    Ok(bytes)
}

/// `sha256:` and the 64 hex digits of the SHA-256 of `bytes`.
fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16));
    "sha256:".chars().chain(hex).collect()
}

#[derive(Debug, Serialize)]
struct Refused<'a> {
    tree: &'a Path,
    problems: usize,
}

/// The envelope for an invalid tree: exit 1, one finding per problem.
fn invalid<O: Write>(
    subcommand: &str,
    tree: &Path,
    problems: &[TreeProblem],
    out: &mut O,
) -> Result<Code, CliError> {
    let findings: Vec<Finding> = problems
        .iter()
        .map(|problem| Finding::new(problem.key(), problem.to_string()))
        .collect();
    let data = Refused {
        tree,
        problems: problems.len(),
    };
    let envelope = Envelope::new(
        CommandName::Grill,
        Some(subcommand),
        exit_codes::GRILL_INVALID,
        data,
    );
    emit(&envelope.with_findings(&findings), out)
}

/// The envelope for a confirmation refused because questions are open: exit 3, one finding
/// per open question.
fn still_open<O: Write>(path: &Path, tree: &DesignTree, out: &mut O) -> Result<Code, CliError> {
    let findings: Vec<Finding> = tree
        .with_status(Status::Open)
        .map(|node| {
            Finding::new(
                format!("grill/open:{}", node.id),
                format!(
                    "{} is still open: answer it or defer it, then ask the human again",
                    node.id
                ),
            )
        })
        .collect();
    let data = Refused {
        tree: path,
        problems: findings.len(),
    };
    let envelope = Envelope::new(
        CommandName::Grill,
        Some("confirm"),
        exit_codes::GRILL_OPEN,
        data,
    );
    emit(&envelope.with_findings(&findings), out)
}

fn emit<D: Serialize, O: Write>(envelope: &Envelope<'_, D>, out: &mut O) -> Result<Code, CliError> {
    envelope.emit(out).map_err(CliError::Output)
}
