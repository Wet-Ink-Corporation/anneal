//! Running a child process to completion within a deadline and with capped output (H-13),
//! shared by `git` (`paths`, and `floor_guard` through `paths::run_git_capped`), `gh`
//! (`dag::tracker::github`'s `GhCli`, which `watch` uses too) and the test command of `task
//! done` (`task::run_command`, which pipes nothing: its child writes straight to anneal's
//! stderr). `perf noise` times its runs with `wait_or_kill_every`.

use std::io::{self, Read};
use std::process::{Child, ExitStatus, Output};
use std::thread::{self, Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

/// How often a running child is checked for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Why waiting for a child failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WaitError {
    /// Its pipes could not be read, or it could not be waited on or killed.
    #[error("waiting for the process")]
    Io(#[source] io::Error),
    /// It did not finish within the timeout, and was killed and reaped.
    #[error("the process did not finish in time")]
    Timeout,
}

/// Waits for `child`, draining its piped stdout and stderr on named scoped threads
/// (`<label>-stdout`, `<label>-stderr`) so a chatty child cannot block on a full pipe. Each
/// stream keeps at most `cap` bytes; the rest is read and dropped. Past `timeout` the child is
/// killed and reaped, and the error is [`WaitError::Timeout`]. Every reader thread is joined
/// before this returns.
///
/// Limit: the readers finish only when every holder of the pipes' write ends has exited. If
/// killing the child fails, or the child started a grandchild that inherited its stdout or
/// stderr, joining the readers blocks until those processes exit, past `timeout`. Std offers
/// no way to abandon a blocked read without detaching the thread (H-13), so this stays a
/// documented limit: use it only for commands, such as `git rev-parse` or `gh api`, that start
/// no long-lived processes. The limit does not apply when nothing is piped (as for `task
/// done`'s test command): there are no readers to join, so a timeout returns once the child
/// itself is killed and reaped, even if a grandchild lives on.
pub(crate) fn wait_with_deadline(
    child: &mut Child,
    timeout: Duration,
    cap: u64,
    label: &str,
) -> Result<Output, WaitError> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    thread::scope(|scope| {
        let out_reader = match spawn_reader(scope, format!("{label}-stdout"), stdout, cap) {
            Ok(handle) => handle,
            Err(error) => return Err(kill_after(child, error)),
        };
        let err_reader = match spawn_reader(scope, format!("{label}-stderr"), stderr, cap) {
            Ok(handle) => handle,
            Err(error) => return Err(kill_after(child, error)),
        };
        let status = wait_or_kill(child, timeout)?;
        Ok(Output {
            status,
            stdout: join_reader(out_reader)?,
            stderr: join_reader(err_reader)?,
        })
    })
}

type Reader<'scope> = ScopedJoinHandle<'scope, io::Result<Vec<u8>>>;

fn spawn_reader<'scope, R: Read + Send + 'scope>(
    scope: &'scope Scope<'scope, '_>,
    name: String,
    pipe: Option<R>,
    cap: u64,
) -> io::Result<Reader<'scope>> {
    thread::Builder::new()
        .name(name)
        .spawn_scoped(scope, move || match pipe {
            Some(pipe) => read_capped(pipe, cap),
            None => Ok(Vec::new()),
        })
}

/// Reads `pipe` to its end, keeping the first `cap` bytes and discarding the rest, so the
/// writer never blocks on a full pipe and memory stays bounded.
pub(crate) fn read_capped<R: Read>(mut pipe: R, cap: u64) -> io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    pipe.by_ref().take(cap).read_to_end(&mut kept)?;
    io::copy(&mut pipe, &mut io::sink())?;
    Ok(kept)
}

fn join_reader(reader: Reader<'_>) -> Result<Vec<u8>, WaitError> {
    reader
        .join()
        .map_err(|_panicked| WaitError::Io(io::Error::other("an output reader thread panicked")))?
        .map_err(WaitError::Io)
}

/// Polls `child` until it exits; past `timeout` kills and reaps it. The deadline is checked
/// before each poll, so an exit first seen after the deadline is a timeout, and a zero
/// timeout always is.
fn wait_or_kill(child: &mut Child, timeout: Duration) -> Result<ExitStatus, WaitError> {
    wait_or_kill_every(child, timeout, POLL_INTERVAL)
}

/// As `wait_or_kill`, checking every `poll`: `perf noise` times its runs with a finer
/// interval than the 5 ms the other callers need.
pub(crate) fn wait_or_kill_every(
    child: &mut Child,
    timeout: Duration,
    poll: Duration,
) -> Result<ExitStatus, WaitError> {
    let started = Instant::now();
    loop {
        if started.elapsed() >= timeout {
            child.kill().map_err(WaitError::Io)?;
            child.wait().map_err(WaitError::Io)?;
            return Err(WaitError::Timeout);
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            // Kill it: the reader threads would otherwise wait on its pipes forever.
            Err(error) => return Err(kill_after(child, error)),
        }
        thread::sleep(poll);
    }
}

/// Kills and reaps `child` after a setup failure, so no reader waits on a live pipe. A failure
/// to kill is reported alongside the original error rather than dropped.
fn kill_after(child: &mut Child, error: io::Error) -> WaitError {
    match child.kill().and_then(|()| child.wait().map(drop)) {
        Ok(()) => WaitError::Io(error),
        Err(kill_error) => WaitError::Io(io::Error::new(
            error.kind(),
            format!("{error}; killing the process also failed: {kill_error}"),
        )),
    }
}
