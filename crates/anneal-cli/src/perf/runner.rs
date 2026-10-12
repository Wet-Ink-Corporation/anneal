//! Running the measured command: the `Runner` port and `ProcessRunner`, its adapter.
//!
//! A run is the command as given (no shell), stdin, stdout and stderr all null, timed by the
//! wall clock (`Instant`) from spawn until its exit is seen, polled every millisecond. Past the
//! timeout it is killed and the run fails. Tests use a fake runner, never a real clock.

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::process::{self, WaitError};

/// How often a running command is checked for its exit: the timing's resolution.
const POLL: Duration = Duration::from_millis(1);

/// Why one run failed.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The command could not be started (not found, not executable).
    #[error("could not start {program}")]
    NotStarted {
        /// The program, as given (lossy).
        program: String,
        /// Why.
        #[source]
        source: io::Error,
    },
    /// The command exited non-zero or was killed by a signal: its time measures a failure.
    #[error("the command failed (exit {})", exit.map_or_else(|| "none: killed by a signal".to_owned(), |code| code.to_string()))]
    Failed {
        /// Its exit code; `None` when a signal killed it.
        exit: Option<i32>,
    },
    /// The command ran past the timeout and was killed.
    #[error("the command ran past its {secs} s timeout and was killed")]
    TimedOut {
        /// The timeout, in seconds.
        secs: u64,
    },
    /// The command could not be waited on or killed.
    #[error("waiting for the command")]
    Wait(#[source] io::Error),
}

/// Runs the measured command once and reports how long it took.
pub(crate) trait Runner {
    /// Runs `command` (program then arguments), killing it past `timeout`.
    ///
    /// # Errors
    /// The [`RunError`] when it cannot start, fails, or runs past `timeout`.
    fn run(&mut self, command: &[OsString], timeout: Duration) -> Result<Duration, RunError>;
}

/// The real runner: a child process in the working directory, timed by the wall clock.
#[derive(Debug, Default)]
pub(crate) struct ProcessRunner;

impl Runner for ProcessRunner {
    fn run(&mut self, command: &[OsString], timeout: Duration) -> Result<Duration, RunError> {
        let Some((program, args)) = command.split_first() else {
            return Err(RunError::NotStarted {
                program: String::new(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "no command"),
            });
        };
        let started = Instant::now();
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| RunError::NotStarted {
                program: lossy(program),
                source,
            })?;
        let status =
            process::wait_or_kill_every(&mut child, timeout, POLL).map_err(
                |error| match error {
                    WaitError::Timeout => RunError::TimedOut {
                        secs: timeout.as_secs(),
                    },
                    WaitError::Io(source) => RunError::Wait(source),
                },
            )?;
        let elapsed = started.elapsed();
        if status.success() {
            Ok(elapsed)
        } else {
            Err(RunError::Failed {
                exit: status.code(),
            })
        }
    }
}

/// `program` as text for a message.
fn lossy(program: &OsStr) -> String {
    program.to_string_lossy().into_owned()
}
