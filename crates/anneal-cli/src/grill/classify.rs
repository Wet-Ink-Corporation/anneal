//! `anneal grill classify --tree <file> --node <id>`: Clef D7 on one open question.
//!
//! D7 (`noul is_decision`) is asked about an open node of a valid design tree through
//! `decide`'s own in-process code (`decide::in_process`, then `client::ask`), so its
//! thresholds, log, modes and shadow count are the command's.
//!
//! Clef gets the question and the tree's goal, each through `hook::safe_prompt` (redacted, at most
//! 1,024 characters); the row keeps neither. Only when D7 is in effect `enforce` (configured,
//! and past its first 50 answered decisions) is Clef's P(yes) written into the node's
//! `is_decision`, so `grill frontier`'s `ask` and `look_up` follow it; in shadow the tree is
//! left exactly as the model wrote it and the skill's prose decides. Clef off, unreachable or
//! misconfigured falls back to the model's own estimate (`fail_open`) and exits 0; an answer
//! whose row could not be logged is an error and is never written. See `docs/grill.md`.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;
use serde_json::json;

use super::tree::{DesignTree, Node, Status, TreeFile};
use super::{GrillError, emit, invalid, read_tree};
use crate::baseline;
use crate::command::CommandName;
use crate::decide::client::{self, Ask, Outcome};
use crate::decide::clock::Clock;
use crate::decide::config::Mode;
use crate::decide::id::DecisionId;
use crate::decide::in_process::{self, PrepareError, Prepared};
use crate::decide::point::{Point, Question};
use crate::decide::state::CompactState;
use crate::decide::transport::Transport;
use crate::decide::{DecideEnv, DecideError};
use crate::envelope::Envelope;
use crate::error::CliError;
use crate::error_chain::reason;
use crate::exit_codes::{self, Code};
use crate::hook::safe_prompt;

/// The subcommand's name in its envelope.
const SUBCOMMAND: &str = "classify";

/// `anneal grill classify`.
#[derive(Debug, Args)]
pub struct ClassifyArgs {
    /// The design tree file
    #[arg(long, value_name = "FILE")]
    tree: PathBuf,
    /// The id of the open question to classify
    #[arg(long, value_name = "ID")]
    node: String,
}

/// What became of the question to Clef, as `data.clef`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ClefOutcome {
    /// No account, or D7's mode is `off`: nothing asked or logged.
    Off,
    /// Clef answered and the row is logged.
    Decided,
    /// Clef gave no usable answer; the row is logged.
    NoAnswer,
    /// The account, token or mode is malformed: nothing asked or logged.
    Misconfigured,
}

/// `data` of `grill classify`.
#[derive(Debug, Serialize)]
struct Classified<'a> {
    tree: &'a Path,
    node: &'a str,
    clef: ClefOutcome,
    /// The decision's id, to label with `anneal decide label`.
    id: Option<&'a DecisionId>,
    /// Clef's P(yes): the question is a decision only the human can make.
    is_decision: Option<f64>,
    action: Option<&'a str>,
    mode: Option<Mode>,
    fallback: Option<&'a str>,
    reason: Option<String>,
    /// Whether `is_decision` was written into the tree (only in effect `enforce`).
    written: bool,
}

impl<'a> Classified<'a> {
    /// Nothing from Clef yet: the outcome `clef`, every other field empty.
    const fn empty(tree: &'a Path, node: &'a str, clef: ClefOutcome) -> Self {
        Self {
            tree,
            node,
            clef,
            id: None,
            is_decision: None,
            action: None,
            mode: None,
            fallback: None,
            reason: None,
            written: false,
        }
    }

    /// What `outcome` reports, nothing written yet.
    fn of(tree: &'a Path, node: &'a str, outcome: &'a Outcome<'_>) -> Self {
        match outcome {
            Outcome::Off(_) => Self::empty(tree, node, ClefOutcome::Off),
            Outcome::Decided(decision) => Self {
                id: Some(&decision.stamp.id),
                is_decision: Some(decision.answer.probability()),
                action: Some(decision.action),
                mode: Some(decision.stamp.mode),
                ..Self::empty(tree, node, ClefOutcome::Decided)
            },
            Outcome::NoAnswer(missed) => Self {
                id: Some(&missed.stamp.id),
                mode: Some(missed.stamp.mode),
                fallback: Some(missed.fallback),
                reason: Some(reason(&missed.failure)),
                ..Self::empty(tree, node, ClefOutcome::NoAnswer)
            },
            Outcome::Misconfigured(refused) => Self {
                fallback: Some(refused.fallback),
                reason: Some(reason(refused.problem)),
                ..Self::empty(tree, node, ClefOutcome::Misconfigured)
            },
        }
    }
}

/// Runs `anneal grill classify` with `env`, writing the envelope to `out`.
pub(super) fn run<O, L, T, C>(
    args: &ClassifyArgs,
    env: &DecideEnv<'_, L, T, C>,
    out: &mut O,
) -> Result<Code, CliError>
where
    O: Write,
    L: Fn(&str) -> Option<OsString>,
    T: Transport,
    C: Clock,
{
    let bytes = read_tree(env.cwd, &args.tree)?;
    let checked = TreeFile::read(&bytes).and_then(|file| {
        // The file keeps the model's node order for a rewrite; the tree sorts its own copy.
        DesignTree::validate(file.clone()).map(|tree| (file, tree))
    });
    let (mut file, tree) = match checked {
        Ok(read) => read,
        Err(problems) => return invalid(SUBCOMMAND, &args.tree, &problems, out),
    };
    let node = open_node(&tree, &args.node, &args.tree)?;
    if matches!(
        in_process::status(Point::D7, &env.lookup),
        in_process::Status::Off
    ) {
        let data = Classified::empty(&args.tree, &args.node, ClefOutcome::Off);
        return emit_classified(&data, out);
    }
    let clef = Prepared::load(&env.lookup).map_err(prepare_error)?;
    let state = state_for(&tree, node)?;
    let ask = Ask {
        point: Point::D7,
        question: Question::Noul,
        options: &[],
        state: &state,
    };
    // `client::ask` turns a row it could not log into an error: an answer that is not in the
    // log is never written into the tree.
    let outcome =
        client::ask(&ask, &clef.setup(), env.transport, env.clock).map_err(DecideError::from)?;
    let mut data = Classified::of(&args.tree, &args.node, &outcome);
    if let Outcome::Decided(decision) = &outcome
        && decision.stamp.mode == Mode::Enforce
        && file.set_is_decision(&node.id, decision.answer.probability())
    {
        save(env.cwd, &args.tree, &file)?;
        data.written = true;
    }
    emit_classified(&data, out)
}

/// The open node `id` of `tree` (read from `path`).
///
/// # Errors
/// A usage error when no node has `id` or the node is not open.
fn open_node<'t>(tree: &'t DesignTree, id: &str, path: &Path) -> Result<&'t Node, CliError> {
    let usage = |reason: String| CliError::Usage { reason };
    let node = tree
        .node(id)
        .ok_or_else(|| usage(format!("{} has no node {id:?}", path.display())))?;
    if node.status == Status::Open {
        Ok(node)
    } else {
        Err(usage(format!(
            "{id} is not open: only an open question is classified"
        )))
    }
}

/// The compact state Clef gets: the phase, the question and the goal, redacted.
fn state_for(tree: &DesignTree, node: &Node) -> Result<CompactState, CliError> {
    let state = json!({
        "phase": "grill",
        "input": safe_prompt(&node.question),
        "goal": safe_prompt(tree.goal()),
    });
    Ok(CompactState::parse(&state.to_string()).map_err(DecideError::from)?)
}

/// Saves `file` over the tree at `path` (relative to `cwd`), atomically.
fn save(cwd: &Path, path: &Path, file: &TreeFile) -> Result<(), GrillError> {
    baseline::write_snapshot_json(&cwd.join(path), file).map_err(|source| GrillError::Write {
        // The error owns the path it names.
        path: path.to_path_buf(),
        source,
    })
}

/// A failure to find the anneal home or read the thresholds, as the command reports it.
fn prepare_error(error: PrepareError) -> CliError {
    match error {
        PrepareError::Home(error) => CliError::Path(error),
        PrepareError::Thresholds(error) => CliError::Decide(error),
    }
}

fn emit_classified<O: Write>(data: &Classified<'_>, out: &mut O) -> Result<Code, CliError> {
    emit(
        &Envelope::new(CommandName::Grill, Some(SUBCOMMAND), exit_codes::OK, data),
        out,
    )
}

#[cfg(test)]
mod tests;
