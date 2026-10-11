//! `anneal decide`: one question to Clef for a decision point, its thresholds applied to the
//! probability Clef returns, and the answer appended to the decision log.
//!
//! `decide --point <D1..D7> --question <noul|choice|score> --state <json> [--option <o>]...
//! [--thresholds <file>]` exits 0 with the action when Clef answered, 3 when Clef is off for
//! the point (no account, or mode `off`; nothing is asked or logged) and 5 when Clef gave no
//! usable answer (unreachable, timed out or a bad response; the row is logged and the
//! envelope names the point's fallback) or the configuration is malformed (nothing is asked
//! or logged; the envelope names the fallback and the problem). `decide label <id> <outcome>`
//! appends an outcome row.
//! See `docs/decide.md`.
//!
//! - [`point`]: the decision points and question types.
//! - [`thresholds`]: `clef-thresholds.json`, models and bands.
//! - [`config`]: account, token and mode from the plugin's `userConfig`.
//! - [`state`]: the compact state.
//! - [`transport`]: the port to Workers AI and the token type.
//! - [`answer`]: reading Clef's probabilities from a response.
//! - [`client`]: one decision, start to row.
//! - [`clock`]: the wall and monotonic clocks, a port.
//! - [`log`]: the decision log and its counts.
//! - [`id`]: decision ids and outcome labels.

pub mod answer;
pub mod client;
pub mod clock;
pub mod config;
pub mod id;
pub mod log;
mod output;
pub mod point;
pub mod state;
pub mod thresholds;
pub mod transport;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTimeError;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::baseline::{self, BaselineError};
use crate::command::CommandName;
use crate::envelope::Envelope;
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths;
use client::{Ask, AskError, Setup};
use clock::Clock;
use config::ClefConfig;
use id::{DecisionId, InvalidLabel, OutcomeLabel};
use log::{DecisionLog, LogError};
use point::{Point, Question};
use state::{CompactState, StateError};
use thresholds::{Thresholds, ThresholdsError};
use transport::Transport;

/// The thresholds file inside the plugin root.
const PLUGIN_THRESHOLDS: [&str; 2] = ["playbooks", "clef-thresholds.json"];

/// `anneal decide`.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct DecideArgs {
    #[command(subcommand)]
    action: Option<DecideAction>,
    /// The decision point
    #[arg(long, value_enum, required = true)]
    point: Option<Point>,
    /// The question type: noul (yes or no), choice (one of --option), score (0 to 1)
    #[arg(long, value_enum, required = true)]
    question: Option<Question>,
    /// The compact state, a JSON object (tool, trimmed input, phase; volatile fields last)
    #[arg(long, value_name = "JSON", required = true)]
    state: Option<String>,
    /// An option of a choice question; repeat for each (2 to 32)
    #[arg(long = "option", value_name = "OPTION")]
    options: Vec<String>,
    /// The thresholds file [default: $CLAUDE_PLUGIN_ROOT/playbooks/clef-thresholds.json, else
    /// the copy built in]
    #[arg(long, value_name = "FILE")]
    thresholds: Option<PathBuf>,
}

/// The `anneal decide` subcommands.
#[derive(Debug, Subcommand)]
pub enum DecideAction {
    /// Attach an outcome label to a logged decision
    Label(LabelArgs),
}

/// `anneal decide label`.
#[derive(Debug, Args)]
pub struct LabelArgs {
    /// The decision's id, from its row or `data.id`
    #[arg(value_name = "ID")]
    id: String,
    /// What turned out to be right (1 to 64 of A-Z a-z 0-9 _ . : -)
    #[arg(value_name = "OUTCOME")]
    outcome: String,
}

/// What `anneal decide` reads from outside: the working directory, the environment, the
/// transport to Workers AI and the clock.
#[derive(Debug)]
pub(crate) struct DecideEnv<'a, L, T, C> {
    /// The working directory, for a relative `--thresholds`.
    pub(crate) cwd: &'a Path,
    /// Reads one environment variable.
    pub(crate) lookup: L,
    /// The port to Workers AI.
    pub(crate) transport: &'a T,
    /// The clocks.
    pub(crate) clock: &'a C,
}

/// Why `anneal decide` could not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum DecideError {
    /// `--state` was refused.
    #[error(transparent)]
    State(#[from] StateError),
    /// `decide label`'s arguments were refused.
    #[error(transparent)]
    Label(#[from] InvalidLabel),
    /// The question could not be asked or recorded.
    #[error(transparent)]
    Ask(#[from] AskError),
    /// The thresholds file could not be read.
    #[error("reading the thresholds file")]
    ThresholdsFile(#[source] BaselineError),
    /// The thresholds file named is missing.
    #[error("no thresholds file at {path}", path = path.display())]
    ThresholdsMissing {
        /// The path looked at.
        path: PathBuf,
    },
    /// The thresholds file is invalid.
    #[error("thresholds {from}")]
    Thresholds {
        /// The file, or `built in`.
        from: String,
        /// What is wrong.
        #[source]
        source: ThresholdsError,
    },
    /// The label row could not be appended.
    #[error(transparent)]
    Log(#[from] LogError),
    /// The system clock is before 1970.
    #[error("reading the clock")]
    Clock(#[source] SystemTimeError),
}

impl DecideError {
    /// The exit code: a usage error (2) for refused arguments, else 70.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::State(_)
            | Self::Label(_)
            | Self::Ask(AskError::Unsupported { .. } | AskError::Options { .. }) => {
                exit_codes::USAGE
            }
            Self::Ask(_)
            | Self::ThresholdsFile(_)
            | Self::ThresholdsMissing { .. }
            | Self::Thresholds { .. }
            | Self::Log(_)
            | Self::Clock(_) => exit_codes::INTERNAL,
        }
    }
}

/// Runs `anneal decide` (or `decide label`) with `env`, writing the envelope to `out`.
pub(crate) fn run<O, L, T, C>(
    args: &DecideArgs,
    env: &DecideEnv<'_, L, T, C>,
    out: &mut O,
) -> Result<Code, CliError>
where
    O: Write,
    L: Fn(&str) -> Option<OsString>,
    T: Transport,
    C: Clock,
{
    let home = paths::anneal_home_from(&env.lookup)?;
    let log = DecisionLog::in_home(&home);
    if let Some(DecideAction::Label(label)) = &args.action {
        return label_decision(label, &log, env.clock, out);
    }
    let (Some(point), Some(question), Some(state)) = (args.point, args.question, &args.state)
    else {
        return Err(CliError::Usage {
            reason: "decide needs --point, --question and --state".to_owned(),
        });
    };
    let state = CompactState::parse(state).map_err(DecideError::from)?;
    let thresholds = load_thresholds(args.thresholds.as_deref(), env.cwd, &env.lookup)?;
    // A malformed configuration is not an internal error: the point falls back (exit 5).
    let config = ClefConfig::from_env(&env.lookup);
    let ask = Ask {
        point,
        question,
        options: &args.options,
        state: &state,
    };
    let setup = Setup {
        thresholds: &thresholds,
        config: config.as_ref(),
        log: &log,
    };
    let outcome = client::ask(&ask, &setup, env.transport, env.clock).map_err(DecideError::from)?;
    output::write_outcome(&ask, &outcome, &log.path(), out)
}

/// The thresholds: `--thresholds` (relative to `cwd`), else the plugin root's file, else the
/// copy built in. A file named either way must exist and be a regular file.
fn load_thresholds(
    flag: Option<&Path>,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<Thresholds, DecideError> {
    read_thresholds(thresholds_path(flag, cwd, lookup).as_deref())
}

/// The thresholds file to read: `--thresholds` (relative to `cwd`), else the plugin root's
/// file; `None` means the copy built in.
pub(crate) fn thresholds_path(
    flag: Option<&Path>,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    flag.map(|path| cwd.join(path)).or_else(|| {
        lookup("CLAUDE_PLUGIN_ROOT")
            .filter(|root| !root.is_empty())
            .map(|root| {
                PLUGIN_THRESHOLDS
                    .iter()
                    .fold(PathBuf::from(root), |p, c| p.join(c))
            })
    })
}

/// Reads the thresholds file at `path`, which must exist and be a regular file, or the copy
/// built in for `None`.
pub(crate) fn read_thresholds(path: Option<&Path>) -> Result<Thresholds, DecideError> {
    let Some(path) = path else {
        return Thresholds::built_in().map_err(|source| DecideError::Thresholds {
            from: "built in".to_owned(),
            source,
        });
    };
    let Some(bytes) = baseline::read_state_bytes(path).map_err(DecideError::ThresholdsFile)? else {
        return Err(DecideError::ThresholdsMissing {
            // The error owns the path it names.
            path: path.to_path_buf(),
        });
    };
    Thresholds::parse(&bytes).map_err(|source| DecideError::Thresholds {
        from: path.display().to_string(),
        source,
    })
}

/// A label row: `{"kind": "label", "id", "outcome", "ts_ms"}`.
#[derive(Debug, Serialize)]
struct LabelRow<'a> {
    kind: &'static str,
    id: &'a DecisionId,
    outcome: &'a OutcomeLabel,
    ts_ms: u64,
}

/// `data` of `decide label`.
#[derive(Debug, Serialize)]
struct LabelData<'a> {
    id: &'a DecisionId,
    outcome: &'a OutcomeLabel,
    log: &'a Path,
}

fn label_decision<O: Write>(
    args: &LabelArgs,
    log: &DecisionLog,
    clock: &impl Clock,
    out: &mut O,
) -> Result<Code, CliError> {
    let id: DecisionId = args.id.parse().map_err(DecideError::from)?;
    let outcome: OutcomeLabel = args.outcome.parse().map_err(DecideError::from)?;
    let ts_ms = clock.unix_millis().map_err(DecideError::Clock)?;
    let row = LabelRow {
        kind: "label",
        id: &id,
        outcome: &outcome,
        ts_ms,
    };
    log.append(&row).map_err(DecideError::from)?;
    let path = log.path();
    let data = LabelData {
        id: &id,
        outcome: &outcome,
        log: &path,
    };
    Envelope::new(
        CommandName::Decide,
        Some("label"),
        exit_codes::DECIDE_DECIDED,
        data,
    )
    .emit(out)
    .map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
