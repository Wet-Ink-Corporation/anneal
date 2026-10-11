//! Running the checks, each bounded (H-13): a deadline per child process, capped output, and
//! a total [`Budget`] under the hook's 15 s timeout.
//!
//! anneal's own checks run as children of the running binary (`anneal floor-guard`, `anneal
//! lint ...`), so each has one deadline however many git calls it makes. `check:fast` writes
//! to a fresh temporary file rather than a pipe: a process it leaves behind cannot hold the
//! hook open, and only the end of its output is read.
//!
//! Limits: on a timeout only the child itself is killed; a process it started (a `git` call
//! under `anneal floor-guard`, anything `check:fast` starts) can outlive it, as for `task
//! done`, since std has no portable process-group kill. On Windows a child's handles are
//! inherited by the processes it starts: a killed `anneal floor-guard` can leave a `git` call
//! holding its output pipe, so joining the readers waits until git exits (bounded by
//! floor-guard's own 60 s per call, past the hook's timeout, after which Claude Code drops the
//! output). A process `check:fast` leaves behind keeps the log file open there, so removing it
//! fails and is noted, and the file stays in the temporary directory.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::error_chain::reason;
use crate::exit_codes;
use crate::process::{self, WaitError};

/// The least time worth starting a process with.
const MIN_GRANT: Duration = Duration::from_millis(100);

/// Most bytes kept of an anneal child's stdout and of its stderr.
const MAX_ANNEAL_OUTPUT: u64 = 4 * 1024 * 1024;

/// Most bytes of `check:fast` output shown: its end, where failures are summed up.
const OUTPUT_TAIL_BYTES: u64 = 2 * 1024;

/// Most findings listed in one note or block reason; the rest are counted.
const MAX_LISTED: usize = 5;

/// The time a check capped at `cap` may take when `elapsed` of the `total` budget is spent:
/// its cap, or what is left when that is less; `None` when too little is left to start one.
pub(crate) fn grant(total: Duration, elapsed: Duration, cap: Duration) -> Option<Duration> {
    let left = total.checked_sub(elapsed)?;
    (left >= MIN_GRANT).then(|| left.min(cap))
}

/// The time the whole run may take, from when it started.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    started: Instant,
    total: Duration,
}

impl Budget {
    /// A budget of `total`, starting now.
    pub(crate) fn start(total: Duration) -> Self {
        Self {
            started: Instant::now(),
            total,
        }
    }

    /// See [`grant`].
    pub(crate) fn grant(&self, cap: Duration) -> Option<Duration> {
        grant(self.total, self.started.elapsed(), cap)
    }

    /// The whole budget, for messages.
    pub(crate) const fn total(&self) -> Duration {
        self.total
    }
}

/// What one check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It passed: nothing to say.
    Pass,
    /// A floor was lowered: the one-line `ERROR` reason that blocks.
    Block(String),
    /// Something to tell the model, without blocking: a failure, or a check that did not run.
    Note(String),
}

/// The part of an anneal envelope the hook reads.
#[derive(Debug, Default, Deserialize)]
struct EnvelopeDto {
    #[serde(default)]
    findings: Vec<FindingDto>,
}

#[derive(Debug, Deserialize)]
struct FindingDto {
    key: String,
    message: String,
}

/// What an anneal child did.
#[derive(Debug)]
struct Ran {
    /// Its exit code; `None` when a signal ended it.
    code: Option<i32>,
    /// Its envelope's findings; `None` when stdout was not an envelope.
    findings: Option<Vec<FindingDto>>,
    /// The first line of its stderr (its `ERROR` line).
    error: String,
}

impl Ran {
    fn exited(&self, code: exit_codes::Code) -> bool {
        self.code == Some(i32::from(code.value()))
    }
}

/// Why a child could not be run to the end.
#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("it could not be started")]
    Spawn(#[source] io::Error),
    #[error(transparent)]
    Wait(#[from] WaitError),
}

/// The running anneal binary, run in a worktree's root.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Anneal<'a> {
    /// The binary.
    pub(crate) exe: &'a Path,
    /// The worktree root, its working directory.
    pub(crate) root: &'a Path,
}

/// Runs `anneal <args>` within `timeout`.
fn run_anneal(anneal: Anneal<'_>, args: &[&str], timeout: Duration) -> Result<Ran, RunError> {
    let mut child = Command::new(anneal.exe)
        .args(args)
        .current_dir(anneal.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
    let output = process::wait_with_deadline(&mut child, timeout, MAX_ANNEAL_OUTPUT, "fast-check")?;
    // An unreadable envelope is reported by `listed` ("its report could not be read"); the
    // exit code alone still decides.
    let findings = serde_json::from_slice::<EnvelopeDto>(&output.stdout)
        .ok()
        .map(|envelope| envelope.findings);
    let error = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    Ok(Ran {
        code: output.status.code(),
        findings,
        error,
    })
}

/// The note for `what`, which did not run to the end.
fn unfinished(what: &str, error: &RunError, timeout: Duration) -> Outcome {
    let note = match error {
        RunError::Wait(WaitError::Timeout) => format!(
            "{what} did not finish within {} s and was stopped; it did not check this batch",
            timeout.as_secs_f32()
        ),
        RunError::Spawn(_) | RunError::Wait(WaitError::Io(_)) => {
            format!("{what} did not run: {}", reason(error))
        }
    };
    Outcome::Note(note)
}

/// The note for `what`, which ended other than with a pass or its findings.
fn failed(what: &str, ran: &Ran) -> Outcome {
    let exit = ran.code.map_or_else(
        || "killed by a signal".to_owned(),
        |code| format!("exit {code}"),
    );
    Outcome::Note(format!("{what} could not check ({exit}): {}", ran.error))
}

/// Up to [`MAX_LISTED`] of `findings`, each through `show`, joined by `"; "`, with a count of
/// the rest.
fn listed(findings: Option<&[FindingDto]>, show: impl Fn(&FindingDto) -> String) -> String {
    let Some(findings) = findings else {
        return "its report could not be read".to_owned();
    };
    let shown: Vec<String> = findings.iter().take(MAX_LISTED).map(show).collect();
    let text = shown.join("; ").replace(['\r', '\n'], " ");
    match findings.len().saturating_sub(MAX_LISTED) {
        0 => text,
        rest => format!("{text} (and {rest} more)"),
    }
}

/// `anneal floor-guard`: a lowered floor (exit 1) blocks; anything else that is not a pass is a
/// note.
pub(crate) fn floor_guard(anneal: Anneal<'_>, timeout: Duration) -> Outcome {
    const WHAT: &str = "anneal floor-guard";
    let ran = match run_anneal(anneal, &["floor-guard"], timeout) {
        Ok(ran) => ran,
        Err(error) => return unfinished(WHAT, &error, timeout),
    };
    if ran.exited(exit_codes::FLOOR_HELD) {
        return Outcome::Pass;
    }
    if !ran.exited(exit_codes::FLOOR_LOWERED) {
        return failed(WHAT, &ran);
    }
    let floors = listed(ran.findings.as_deref(), |finding| {
        format!("{}: {}", finding.key, finding.message)
    });
    Outcome::Block(format!(
        "ERROR fast-check: {WHAT} reports a lowered floor: {floors}; restore it (`{WHAT}` lists \
         every one)"
    ))
}

/// `anneal lint <args>`, labelled `what`: findings (exit 1) are a note.
pub(crate) fn lint(anneal: Anneal<'_>, what: &str, args: &[&str], timeout: Duration) -> Outcome {
    let ran = match run_anneal(anneal, args, timeout) {
        Ok(ran) => ran,
        Err(error) => return unfinished(what, &error, timeout),
    };
    if ran.exited(exit_codes::LINT_CLEAN) {
        return Outcome::Pass;
    }
    if !ran.exited(exit_codes::LINT_FINDINGS) {
        return failed(what, &ran);
    }
    // Each shown message is copied into the note's text.
    let found = listed(ran.findings.as_deref(), |finding| finding.message.clone());
    Outcome::Note(format!("{what}: {found}"))
}

/// Runs the `check:fast` command `argv` in `root` within `timeout`, without a shell: a pass, or
/// a note with its exit and the end of its output.
pub(crate) fn check_fast(argv: &[String], root: &Path, timeout: Duration) -> Outcome {
    let shown = format!("check_fast `{}`", argv.join(" "));
    let Some((program, rest)) = argv.split_first() else {
        return Outcome::Note(format!("{shown} names no program"));
    };
    let log = match Log::create() {
        Ok(log) => log,
        Err(error) => {
            return Outcome::Note(format!(
                "{shown} did not run: its output file could not be created ({error})"
            ));
        }
    };
    let ran = log.run(Command::new(program).args(rest).current_dir(root), timeout);
    let tail = log.tail();
    let removed = log.remove();
    let mut outcome = match ran {
        Ok(status) if status.success() => Outcome::Pass,
        Ok(status) => {
            let exit = status.code().map_or_else(
                || "killed by a signal".to_owned(),
                |code| format!("exit {code}"),
            );
            let output = match tail {
                Ok(tail) => tail,
                Err(error) => format!("(its output could not be read: {error})"),
            };
            Outcome::Note(format!(
                "{shown} failed ({exit}); the end of its output:\n{output}"
            ))
        }
        Err(error) => unfinished(&shown, &error, timeout),
    };
    if let Err(error) = removed {
        let note = format!("{shown}: its output file could not be removed ({error})");
        outcome = match outcome {
            Outcome::Note(text) => Outcome::Note(format!("{text}\n{note}")),
            Outcome::Pass | Outcome::Block(_) => Outcome::Note(note),
        };
    }
    outcome
}

/// A fresh file in the temporary directory that a child writes its stdout and stderr to.
#[derive(Debug)]
struct Log {
    path: PathBuf,
    file: File,
}

impl Log {
    fn create() -> io::Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        let name = format!("anneal-fast-check-{}-{nanos}.log", std::process::id());
        let path = std::env::temp_dir().join(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path, file })
    }

    /// Runs `command` with stdin closed and its output in this file, killed past `timeout`.
    fn run(&self, command: &mut Command, timeout: Duration) -> Result<ExitStatus, RunError> {
        let stdout = self.file.try_clone().map_err(RunError::Spawn)?;
        let stderr = self.file.try_clone().map_err(RunError::Spawn)?;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .map_err(RunError::Spawn)?;
        // Nothing is piped, so no reader waits on a process the command left behind.
        let output = process::wait_with_deadline(&mut child, timeout, 0, "check-fast")?;
        Ok(output.status)
    }

    /// The last [`OUTPUT_TAIL_BYTES`] written, trimmed.
    fn tail(&self) -> io::Result<String> {
        let mut file = &self.file;
        let len = file.seek(SeekFrom::End(0))?;
        let start = len.saturating_sub(OUTPUT_TAIL_BYTES);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(OUTPUT_TAIL_BYTES).read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim();
        Ok(if start > 0 {
            format!("...{text}")
        } else {
            text.to_owned()
        })
    }

    /// Closes and deletes the file.
    fn remove(self) -> io::Result<()> {
        drop(self.file);
        fs::remove_file(&self.path)
    }
}
