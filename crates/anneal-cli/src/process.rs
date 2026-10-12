//! Running a child process to completion within a deadline and with capped output (H-13),
//! shared by `git` (`paths`, and `floor_guard` through `paths::run_git_capped`), `gh`
//! (`dag::tracker::github`'s `GhCli`, which `watch` uses too) and the test command of `task
//! done` (`task::run_command`, which pipes nothing: its child writes straight to anneal's
//! stderr). `perf noise` times its runs with `wait_or_kill_every`, and the weigh-in bridge
//! (`weighin::ProcessRunner`) pipes a batch to `weighin` with `wait_with_input`.

use std::io::{self, Read, Write};
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
    wait_piped(child, None, Limits { timeout, cap }, label)
}

/// How long a child may run and how much of each output stream is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Past this the child is killed and reaped.
    pub(crate) timeout: Duration,
    /// The bytes kept of each of stdout and stderr; the rest is read and dropped.
    pub(crate) cap: u64,
}

/// As [`wait_with_deadline`], first handing `input` to the child's piped stdin on a named scoped
/// thread (`<label>-stdin`) that closes the pipe when done, so the child sees the end of its
/// input and a child that never reads cannot block the caller. A child that exits without
/// reading it all is no error: its exit status says how it went. The writer is joined before
/// this returns; past the timeout the killed child's pipe ends the write.
pub(crate) fn wait_with_input(
    child: &mut Child,
    input: &[u8],
    limits: Limits,
    label: &str,
) -> Result<Output, WaitError> {
    wait_piped(child, Some(input), limits, label)
}

/// With `input`, writes it to the child's stdin when that is piped (without, stdin is left to
/// the caller); drains its stdout and stderr, and waits within the limits.
fn wait_piped(
    child: &mut Child,
    input: Option<&[u8]>,
    Limits { timeout, cap }: Limits,
    label: &str,
) -> Result<Output, WaitError> {
    let stdin = input.and_then(|_| child.stdin.take());
    let input = input.unwrap_or_default();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    thread::scope(|scope| {
        let writer = match spawn_writer(scope, format!("{label}-stdin"), stdin, input) {
            Ok(handle) => handle,
            Err(error) => return Err(kill_after(child, error)),
        };
        let out_reader = match spawn_reader(scope, format!("{label}-stdout"), stdout, cap) {
            Ok(handle) => handle,
            Err(error) => return Err(kill_after(child, error)),
        };
        let err_reader = match spawn_reader(scope, format!("{label}-stderr"), stderr, cap) {
            Ok(handle) => handle,
            Err(error) => return Err(kill_after(child, error)),
        };
        let status = wait_or_kill(child, timeout)?;
        join_writer(writer)?;
        Ok(Output {
            status,
            stdout: join_reader(out_reader)?,
            stderr: join_reader(err_reader)?,
        })
    })
}

type Writer<'scope> = ScopedJoinHandle<'scope, io::Result<()>>;

fn spawn_writer<'scope, W: Write + Send + 'scope>(
    scope: &'scope Scope<'scope, '_>,
    name: String,
    pipe: Option<W>,
    input: &'scope [u8],
) -> io::Result<Writer<'scope>> {
    thread::Builder::new()
        .name(name)
        .spawn_scoped(scope, move || {
            let Some(mut pipe) = pipe else {
                return Ok(());
            };
            match pipe.write_all(input) {
                // The child exited, or closed its stdin, before reading it all.
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
                // Dropping the pipe closes it: the child reads the end of its input.
                written => written,
            }
        })
}

fn join_writer(writer: Writer<'_>) -> Result<(), WaitError> {
    writer
        .join()
        .map_err(|_panicked| WaitError::Io(io::Error::other("the input writer thread panicked")))?
        .map_err(WaitError::Io)
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

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use super::{Limits, wait_with_input};

    /// Thirty seconds and 1 KiB of each stream: ample for `git` on any runner.
    const LIMITS: Limits = Limits {
        timeout: Duration::from_secs(30),
        cap: 1024,
    };

    #[test]
    fn the_child_reads_its_input_from_stdin_and_its_output_is_kept() {
        // git is on every platform the tests run on, and hashes exactly what stdin held.
        let mut child = Command::new("git")
            .args(["hash-object", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("git starts");

        let output = wait_with_input(&mut child, b"hello\n", LIMITS, "git").expect("git finishes");

        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"ce013625030ba8dba906f756967f9e9ca394464a\n");
    }

    #[test]
    fn a_child_that_never_reads_its_input_still_finishes() {
        // `git --version` ignores stdin; a large input must not block the caller.
        let mut child = Command::new("git")
            .arg("--version")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("git starts");

        let output = wait_with_input(&mut child, &vec![b'x'; 1024 * 1024], LIMITS, "git")
            .expect("git finishes");

        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.starts_with(b"git version"), "{output:?}");
    }
}
