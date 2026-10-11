//! `anneal repro record|leftovers`: the red reproduction diagnose mode waits for, and the
//! debug instrumentation it must not leave behind.
//!
//! `record [--id <id>] [--timeout <secs>] [--session <id>] -- <command>` runs the command as
//! `task done` runs a test command (`task::run_command`: no shell, its output on anneal's
//! stderr, killed past `--timeout`, 1800 s by default). Only a red run, one that exited
//! non-zero or was killed by a signal, appends `repro_red` (`task` the id, `test` the command),
//! with the envelope's `worktree` set to the worktree key `anneal mode` writes
//! (`mode::scope::worktree_key`), so the preconditions hook unlocks only that worktree. A run
//! that exits 0 reproduces nothing and a run that times out is a hang, not a reproduction:
//! each records nothing, opens no ledger and exits with its own code. There is no `--store`:
//! like `anneal mode`, it writes to the ledger the hook reads.
//!
//! `leftovers` lists every `[DEBUG-<id>]` tag in the worktree ([`debug_leftovers`]).
//!
//! - `leftovers`: the scan.

pub mod leftovers;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use anneal_ledger::event::ReproRed;
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::mode::scope;
use crate::paths::{self, GIT_TIMEOUT, PathError, StoreLocation, StoreSource};
use crate::task::{self, DEFAULT_TIMEOUT_SECS, Ending, Run, TaskError};

pub use leftovers::{Leftover, LeftoverError, LeftoverScan, debug_leftovers};

/// The id `repro record` records when `--id` is not given.
pub const DEFAULT_ID: &str = "repro";

/// `anneal repro`.
#[derive(Debug, Args)]
pub struct ReproArgs {
    #[command(subcommand)]
    action: ReproAction,
}

/// The `anneal repro` subcommands.
#[derive(Debug, Subcommand)]
pub enum ReproAction {
    /// Run a reproduction; only if it fails (exits non-zero), record `repro_red` for this
    /// worktree, which unlocks source edits in diagnose mode
    Record(RecordArgs),
    /// List every `[DEBUG-<id>]` tag left in the worktree's files, as `file:line`
    Leftovers,
}

/// `anneal repro record`. There is no `--store`: the reproduction goes to the ledger the
/// preconditions hook reads.
#[derive(Debug, Args)]
pub struct RecordArgs {
    /// The task or bug the reproduction belongs to
    #[arg(long, value_name = "ID", default_value = DEFAULT_ID)]
    id: String,
    /// Kill the command after this many seconds (at least 1); a timeout records nothing
    #[arg(
        long,
        value_name = "SECS",
        default_value_t = DEFAULT_TIMEOUT_SECS,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    timeout: u64,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
    /// The reproduction command and its arguments, after `--`; run as given, without a shell
    #[arg(last = true, required = true, value_name = "COMMAND")]
    command: Vec<OsString>,
}

/// Why `anneal repro` could not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum ReproError {
    /// The working directory is not in a git worktree.
    #[error("anneal repro needs a git worktree")]
    NotAWorktree(#[source] PathError),
    /// The command could not be started (not found, not executable).
    #[error("repro: could not start {program}")]
    Spawn {
        /// The program as given.
        program: String,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// The command started but could not be waited on or killed.
    #[error("repro: waiting for {program}")]
    Wait {
        /// The program as given.
        program: String,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// The worktree's files could not be scanned for leftovers.
    #[error("repro leftovers")]
    Leftovers(#[from] LeftoverError),
}

impl ReproError {
    /// The documented exit code: a usage error (2) outside a worktree, 2 when the command could
    /// not be started (nothing ran or was recorded), else 70.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::NotAWorktree(_) => exit_codes::USAGE,
            Self::Spawn { .. } => exit_codes::REPRO_NOT_STARTED,
            Self::Wait { .. } | Self::Leftovers(_) => exit_codes::INTERNAL,
        }
    }
}

impl From<TaskError> for ReproError {
    fn from(error: TaskError) -> Self {
        match error {
            TaskError::Spawn { program, source } => Self::Spawn { program, source },
            TaskError::Wait { program, source } => Self::Wait { program, source },
        }
    }
}

/// How a reproduction run ended, as `repro record` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    /// It exited non-zero or was killed by a signal: the failure is reproduced.
    Red,
    /// It exited 0: nothing is reproduced.
    Green,
    /// It ran past its timeout and was killed: a hang, not a reproduction.
    TimedOut,
}

/// Runs `anneal repro <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &ReproArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    match &args.action {
        ReproAction::Record(record_args) => record(record_args, cwd, lookup, out),
        ReproAction::Leftovers => leftovers_in(cwd, out),
    }
}

/// The top-level directory of the worktree holding `cwd`; outside one, a usage error.
fn worktree_root(cwd: &Path) -> Result<String, CliError> {
    paths::worktree_root_within(cwd, GIT_TIMEOUT).map_err(|error| match error {
        PathError::NotARepository { .. } => CliError::Repro(ReproError::NotAWorktree(error)),
        other => CliError::Path(other),
    })
}

/// A reproduction run of `repro record`: the id and command it reports, and the `repro_red`
/// envelope it appends if the run is red.
struct Attempt<'a> {
    id: &'a str,
    command: &'a str,
    envelope: EventEnvelope,
}

fn record<O: Write>(
    args: &RecordArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let words = task::utf8_command(&args.command)?;
    let [program, rest @ ..] = words.as_slice() else {
        return Err(CliError::Usage {
            reason: "a reproduction command is required after --".to_owned(),
        });
    };
    let command = words.join(" ");
    let root = worktree_root(cwd)?;
    let event = LedgerEvent::ReproRed(ReproRed {
        // The event owns what it records; the command is also reported in `data`.
        task: args.id.clone(),
        test: command.clone(),
    });
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.worktree = Some(scope::worktree_key(&root));
    envelope.session_id.clone_from(&args.session);
    // The id, worktree and session become ledger tags; check them before anything runs, so a
    // value the store would refuse is a usage error that runs nothing and leaves no file.
    if let Err(error) = envelope.tags() {
        return Err(CliError::Usage {
            reason: format!(
                "the --id, --session and worktree are recorded as ledger tags: {error}"
            ),
        });
    }
    let run = task::run_command(program, rest, cwd, Duration::from_secs(args.timeout))
        .map_err(ReproError::from)?;
    let attempt = Attempt {
        id: &args.id,
        command: &command,
        envelope,
    };
    finish(
        attempt,
        &run,
        || paths::ledger_store(None, lookup, cwd),
        out,
    )
}

/// `data` of `repro record`.
#[derive(Debug, Serialize)]
struct RecordData<'a> {
    id: &'a str,
    command: &'a str,
    worktree: Option<&'a str>,
    outcome: Outcome,
    /// The command's exit code; none after a signal or the timeout.
    exit: Option<i32>,
    duration_ms: u64,
    /// The timeout the command ran past, when it did.
    timeout_secs: Option<u64>,
    /// Whether `repro_red` was appended.
    recorded: bool,
    /// Where it was appended, when it was.
    ledger: Option<Appended<'a>>,
}

/// Where `repro_red` was appended.
#[derive(Debug, Serialize)]
struct Appended<'a> {
    store: &'a Path,
    source: StoreSource,
    position: u64,
    event: &'a EventEnvelope,
}

/// Appends the attempt's envelope (stamped now) to the ledger `locate` resolves only when
/// `run` is red, and writes the result to `out`.
fn finish<O: Write>(
    attempt: Attempt<'_>,
    run: &Run,
    locate: impl FnOnce() -> Result<StoreLocation, PathError>,
    out: &mut O,
) -> Result<Code, CliError> {
    let Attempt {
        id,
        command,
        mut envelope,
    } = attempt;
    let (outcome, code, findings) = verdict(id, run.ending);
    let appended = match outcome {
        Outcome::Red => {
            envelope.ts = Timestamp::now()?;
            let location = locate()?;
            let position = Store::open(&location.path)?.append(&envelope)?;
            Some((location, position))
        }
        Outcome::Green | Outcome::TimedOut => None,
    };
    let (exit, timeout_secs) = match run.ending {
        Ending::Exited(status) => (status.code(), None),
        Ending::TimedOut { after } => (None, Some(after.as_secs())),
    };
    let data = RecordData {
        id,
        command,
        worktree: envelope.worktree.as_deref(),
        outcome,
        exit,
        // Saturates: a run of over 584 million years.
        duration_ms: u64::try_from(run.duration.as_millis()).unwrap_or(u64::MAX),
        timeout_secs,
        recorded: appended.is_some(),
        ledger: appended.as_ref().map(|(location, position)| Appended {
            store: &location.path,
            source: location.source,
            position: *position,
            event: &envelope,
        }),
    };
    Envelope::new(CommandName::Repro, Some("record"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// What a run of the reproduction for `id` that ended as `ending` is, its exit code and its
/// findings. Only a red run (non-zero, or killed by a signal) is recorded.
fn verdict(id: &str, ending: Ending) -> (Outcome, Code, Vec<Finding>) {
    match ending {
        Ending::Exited(status) if !status.success() => {
            (Outcome::Red, exit_codes::REPRO_OK, Vec::new())
        }
        Ending::Exited(_) => (
            Outcome::Green,
            exit_codes::REPRO_NOT_RED,
            vec![Finding::new(
                format!("repro/not-red:{id}"),
                "the command exited 0, so it reproduces nothing; nothing is recorded and diagnose \
                 mode still denies source edits. fix: make a test or command that fails because \
                 of the bug, then record it",
            )],
        ),
        Ending::TimedOut { after } => (
            Outcome::TimedOut,
            exit_codes::REPRO_TIMED_OUT,
            vec![Finding::new(
                format!("repro/timed-out:{id}"),
                format!(
                    "the command ran past its {}s timeout and was killed; a hang is not a \
                     reproduction, so nothing is recorded. fix: make the reproduction fail fast \
                     (a test with its own timeout), or raise --timeout",
                    after.as_secs()
                ),
            )],
        ),
    }
}

/// `data` of `repro leftovers`.
#[derive(Debug, Serialize)]
struct LeftoversData<'a> {
    /// The worktree scanned.
    root: &'a str,
    #[serde(flatten)]
    found: &'a LeftoverScan,
}

fn leftovers_in<O: Write>(cwd: &Path, out: &mut O) -> Result<Code, CliError> {
    let root = worktree_root(cwd)?;
    let found = debug_leftovers(Path::new(&root)).map_err(ReproError::from)?;
    let mut findings: Vec<Finding> = found
        .leftovers()
        .iter()
        .map(|leftover| {
            let (file, line) = (leftover.file(), leftover.line());
            Finding::new(
                format!("repro/leftover:{file}:{line}"),
                format!(
                    "{file}:{line}: a [DEBUG-<id>] tag is left. fix: remove the debug \
                     instrumentation on this line"
                ),
            )
        })
        .collect();
    if found.omitted() > 0 {
        findings.push(Finding::new(
            "repro/leftover-omitted",
            format!("{} more tagged lines are not listed", found.omitted()),
        ));
    }
    let code = if found.is_clean() {
        exit_codes::REPRO_OK
    } else {
        exit_codes::REPRO_LEFTOVERS
    };
    let data = LeftoversData {
        root: &root,
        found: &found,
    };
    Envelope::new(CommandName::Repro, Some("leftovers"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
