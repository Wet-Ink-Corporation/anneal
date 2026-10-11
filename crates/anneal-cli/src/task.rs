//! `anneal task done <id> -- <command...>`: completion is recorded only on a green test run.
//!
//! The test command runs as given (no shell), in the working directory, with stdin closed and
//! both of its output streams on anneal's stderr, so anneal's stdout stays one JSON envelope
//! and nothing is buffered. It has `--timeout` seconds (default 1800); past that it is killed.
//!
//! Only after the command has finished is the ledger opened: a run that exits 0 appends
//! `test_run` (green) and then `task_done`; any other run (a non-zero exit, a kill by a signal,
//! the timeout) appends only a red `test_run` and exits 1. A command that cannot be started
//! appends nothing and exits 2 with one `ERROR` line.
//!
//! Limit: on a timeout only the command's own process is killed. A process it started (a test
//! binary under `cargo test`) can outlive it; std has no portable process-group kill.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anneal_ledger::event::{TaskDone, TestOutcome, TestRun};
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, TagError, Timestamp};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::mode::scope;
use crate::paths::{self, PathError, StoreSource};
use crate::process::{self, WaitError};

/// The default `--timeout`, in seconds: 30 minutes.
pub const DEFAULT_TIMEOUT_SECS: u64 = 1800;

/// `anneal task`.
#[derive(Debug, Args)]
pub struct TaskArgs {
    #[command(subcommand)]
    action: TaskAction,
}

/// The `anneal task` subcommands.
#[derive(Debug, Subcommand)]
pub enum TaskAction {
    /// Run the task's test command; only if it exits 0, record the task as done
    Done(DoneArgs),
}

/// `anneal task done`.
#[derive(Debug, Args)]
pub struct DoneArgs {
    /// The task id
    #[arg(value_name = "ID")]
    id: String,
    /// Kill the test command after this many seconds (at least 1)
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
    /// Use this ledger file instead of the resolved one
    #[arg(long, value_name = "FILE")]
    store: Option<PathBuf>,
    /// The test command and its arguments, after `--`; run as given, without a shell
    #[arg(last = true, required = true, value_name = "COMMAND")]
    command: Vec<OsString>,
}

/// Why `anneal task done` could not run its test command.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    /// The command could not be started (not found, not executable).
    #[error("task: could not start {program}")]
    Spawn {
        /// The program as given.
        program: String,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// The command started but could not be waited on or killed.
    #[error("task: waiting for {program}")]
    Wait {
        /// The program as given.
        program: String,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

impl TaskError {
    /// The documented exit code: 2 when the command could not be started (nothing was run or
    /// recorded), 70 when it could not be waited on.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Spawn { .. } => exit_codes::TASK_NOT_STARTED,
            Self::Wait { .. } => exit_codes::INTERNAL,
        }
    }
}

/// How a test command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// It exited by itself (or was killed by a signal, with no exit code).
    Exited(ExitStatus),
    /// It ran past its deadline, `after`, and was killed.
    TimedOut {
        /// The deadline it was given.
        after: Duration,
    },
}

/// A finished test command run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    /// How it ended.
    pub(crate) ending: Ending,
    /// How long it ran.
    pub(crate) duration: Duration,
}

impl Run {
    /// True only for an exit with status 0.
    pub(crate) fn is_green(&self) -> bool {
        matches!(self.ending, Ending::Exited(status) if status.success())
    }

    /// The `test_run` event for this run of `command` for `task`.
    pub(crate) fn event(&self, task: &str, command: &str) -> TestRun {
        let (exit, timeout_secs) = match self.ending {
            Ending::Exited(status) => (status.code(), None),
            Ending::TimedOut { after } => (None, Some(after.as_secs())),
        };
        TestRun {
            // The event owns what it records.
            task: Some(task.to_owned()),
            command: command.to_owned(),
            outcome: if self.is_green() {
                TestOutcome::Green
            } else {
                TestOutcome::Red
            },
            exit,
            // Saturates: a run of over 584 million years.
            duration_ms: Some(u64::try_from(self.duration.as_millis()).unwrap_or(u64::MAX)),
            timeout_secs,
        }
    }

    /// Why the run is red, in one line.
    fn red_message(&self) -> String {
        let why = match self.ending {
            Ending::TimedOut { after } => {
                format!("ran past its {}s timeout and was killed", after.as_secs())
            }
            Ending::Exited(status) => match status.code() {
                Some(code) => format!("exited {code}"),
                None => "was killed by a signal".to_owned(),
            },
        };
        format!("the test command {why}; the task is not recorded as done")
    }
}

/// Runs `program` with `args` in `cwd`: stdin closed, stdout and stderr on this process's
/// stderr. Past `timeout` it is killed and the run is [`Ending::TimedOut`].
///
/// # Errors
/// [`TaskError::Spawn`] when it cannot be started; [`TaskError::Wait`] when it cannot be
/// waited on or killed.
pub(crate) fn run_command(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout: Duration,
) -> Result<Run, TaskError> {
    let started = Instant::now();
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(io::stderr()))
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|source| TaskError::Spawn {
            // The error owns the name it reports.
            program: program.to_owned(),
            source,
        })?;
    // Nothing is piped, so nothing is captured: the cap is moot.
    let ending = match process::wait_with_deadline(&mut child, timeout, 0, "test") {
        Ok(output) => Ending::Exited(output.status),
        Err(WaitError::Timeout) => Ending::TimedOut { after: timeout },
        Err(WaitError::Io(source)) => {
            return Err(TaskError::Wait {
                // The error owns the name it reports.
                program: program.to_owned(),
                source,
            });
        }
    };
    Ok(Run {
        ending,
        duration: started.elapsed(),
    })
}

/// Runs `anneal task <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &TaskArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    match &args.action {
        TaskAction::Done(done_args) => done(done_args, cwd, lookup, out),
    }
}

#[derive(Debug, Serialize)]
struct Recorded<'a> {
    position: u64,
    event: &'a EventEnvelope,
}

#[derive(Debug, Serialize)]
struct Data<'a> {
    task: &'a str,
    command: &'a str,
    store: &'a Path,
    source: StoreSource,
    events: Vec<Recorded<'a>>,
}

fn done<O: Write>(
    args: &DoneArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let words = utf8_command(&args.command)?;
    let [program, rest @ ..] = words.as_slice() else {
        return Err(CliError::Usage {
            reason: "a test command is required after --".to_owned(),
        });
    };
    let command = words.join(" ");
    let worktree = worktree_of(cwd)?;
    let stamp = Stamp {
        args,
        worktree: worktree.as_deref(),
    };
    // The task, session and worktree become ledger tags; check them before anything runs, so
    // an id the store would refuse is a usage error that runs nothing and leaves no file.
    if let Err(error) = stamp.stamp(args.task_done())?.tags() {
        return Err(CliError::Usage {
            reason: unrecordable(&error),
        });
    }
    let run = run_command(program, rest, cwd, Duration::from_secs(args.timeout))?;
    // Only now, with the command finished, is the ledger opened.
    let location = paths::ledger_store(args.store.as_deref(), lookup, cwd)?;
    let events = record(stamp, &run, &command, &location.path)?;
    let data = Data {
        task: &args.id,
        command: &command,
        store: &location.path,
        source: location.source,
        events: events
            .iter()
            .map(|(position, event)| Recorded {
                position: *position,
                event,
            })
            .collect(),
    };
    let (code, findings) = if run.is_green() {
        (exit_codes::TASK_GREEN, Vec::new())
    } else {
        let key = format!("task/red:{}", args.id);
        let red = Finding::new(key, run.red_message());
        (exit_codes::TASK_TEST_RED, vec![red])
    };
    Envelope::new(CommandName::Task, Some("done"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

impl DoneArgs {
    /// The `task_done` event for this task.
    fn task_done(&self) -> LedgerEvent {
        LedgerEvent::TaskDone(TaskDone {
            // The event owns what it records.
            task: self.id.clone(),
        })
    }
}

/// What every event of one `task done` run carries besides its payload.
struct Stamp<'a> {
    args: &'a DoneArgs,
    /// The worktree key of the working directory's git worktree, if it is in one.
    worktree: Option<&'a str>,
}

impl Stamp<'_> {
    /// `event` in an envelope stamped now, with this run's session and worktree.
    fn stamp(&self, event: LedgerEvent) -> Result<EventEnvelope, CliError> {
        let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
        envelope.session_id.clone_from(&self.args.session);
        envelope.worktree = self.worktree.map(str::to_owned);
        Ok(envelope)
    }
}

impl<'a> From<&'a DoneArgs> for Stamp<'a> {
    /// The stamp of a run outside any git worktree.
    fn from(args: &'a DoneArgs) -> Self {
        Self {
            args,
            worktree: None,
        }
    }
}

/// The worktree key (`mode::scope::worktree_key`) of the git worktree holding `cwd`, which the
/// Stop hook selects test runs by; `None` outside one.
fn worktree_of(cwd: &Path) -> Result<Option<String>, CliError> {
    match paths::worktree_root_within(cwd, paths::GIT_TIMEOUT) {
        Ok(root) => Ok(Some(scope::worktree_key(&root))),
        Err(PathError::NotARepository { .. }) => Ok(None),
        Err(other) => Err(CliError::Path(other)),
    }
}

/// Appends `run`'s `test_run` to the ledger at `store`, then `task_done` only when the run is
/// green, and returns each event with its position.
fn record<'a>(
    stamp: impl Into<Stamp<'a>>,
    run: &Run,
    command: &str,
    store: &Path,
) -> Result<Vec<(u64, EventEnvelope)>, CliError> {
    let stamp = stamp.into();
    let store = Store::open(store)?;
    let args = stamp.args;
    let test_run = stamp.stamp(LedgerEvent::TestRun(run.event(&args.id, command)))?;
    let mut events = vec![(store.append(&test_run)?, test_run)];
    if run.is_green() {
        let done = stamp.stamp(args.task_done())?;
        events.push((store.append(&done)?, done));
    }
    Ok(events)
}

/// The command's words as UTF-8, as the ledger records them (`repro record` shares it).
pub(crate) fn utf8_command(command: &[OsString]) -> Result<Vec<&str>, CliError> {
    command
        .iter()
        .map(|word| {
            word.to_str().ok_or_else(|| CliError::Usage {
                reason: "the test command must be UTF-8: it is recorded in the ledger".to_owned(),
            })
        })
        .collect()
}

/// The one-line usage reason for a task id or `--session` the ledger refuses as a tag.
fn unrecordable(error: &TagError) -> String {
    let prefix = "the task id and --session are recorded as ledger tags";
    match std::error::Error::source(error) {
        Some(cause) => format!("{prefix}: {error}: {cause}"),
        None => format!("{prefix}: {error}"),
    }
}

#[cfg(test)]
mod tests;
