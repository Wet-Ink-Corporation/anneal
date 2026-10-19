//! Running a child process to completion within a deadline and with capped output (H-13),
//! shared by `git` (`paths`, and `floor_guard` through `paths::run_git_capped`), `gh`
//! (`dag::tracker::github`'s `GhCli`, which `watch` uses too) and the test command of `task
//! done` (`task::run_command`, which pipes nothing: its child writes straight to anneal's
//! stderr). `perf noise` times its runs with `wait_or_kill_every`, and the weigh-in bridge
//! (`weighin::ProcessRunner`) pipes a batch to `weighin` with `wait_with_input`.

use std::io::{self, Read, Write};
use std::process::{Child, ExitStatus, Output};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How often a running child is checked for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Why waiting for a child failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WaitError {
    /// Its pipes could not be read, or it could not be waited on or killed.
    #[error("waiting for the process")]
    Io(#[source] io::Error),
    /// It, or a process still holding its pipes, did not finish within the timeout. The child
    /// itself was killed if still running, and reaped.
    #[error("the process did not finish in time")]
    Timeout,
}

/// Waits for `child`, draining its piped stdout and stderr on named threads (`<label>-stdout`,
/// `<label>-stderr`) so a chatty child cannot block on a full pipe. Each stream keeps at most
/// `cap` bytes; the rest is read and dropped. Past `timeout` the child is killed and reaped,
/// and the error is [`WaitError::Timeout`].
///
/// The output is complete only once every holder of the pipes' write ends has closed them, so
/// a child that exits while a grandchild it started still holds its stdout or stderr is a
/// timeout too, at the deadline. Either way this returns within `timeout` plus about 100 ms:
/// after a kill the threads get that long to see the pipes close and are joined; one a
/// grandchild still blocks is left to end on its own, holding only its pipe end and at most
/// `cap` bytes, until that grandchild closes the pipe or exits, or this process exits. Only the
/// child is killed, not its descendants: std cannot kill a process group without `unsafe`.
/// With nothing piped (as for `task done`'s test command) there are no threads, and a timeout
/// returns once the child itself is killed and reaped.
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

/// As [`wait_with_deadline`], first handing `input` to the child's piped stdin on a named
/// thread (`<label>-stdin`) that closes the pipe when done, so the child sees the end of its
/// input and a child that never reads cannot block the caller. A child that exits without
/// reading it all is no error: its exit status says how it went. The writer is joined, or left
/// to end on its own past the timeout, as the readers are.
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
    let started = Instant::now();
    let workers = match Workers::spawn(child, input, cap, label) {
        Ok(workers) => workers,
        Err(error) => return Err(kill_after(child, error)),
    };
    let status = match wait_or_kill(child, timeout) {
        Ok(status) => status,
        Err(error) => {
            // Killed and reaped: its pipes close now, unless a grandchild holds them too.
            poll_until(ABANDON_GRACE, || workers.are_finished());
            workers.abandon();
            return Err(error);
        }
    };
    // The child exited, but a grandchild may still hold its pipes: wait no longer than the
    // deadline for them to close.
    let left = timeout.saturating_sub(started.elapsed());
    if !poll_until(left, || workers.are_finished()) {
        workers.abandon();
        return Err(WaitError::Timeout);
    }
    workers.join(status)
}

/// How long the pipe threads of a killed child get to see its pipes close before the wait
/// returns without them.
const ABANDON_GRACE: Duration = Duration::from_millis(100);

/// The error when the stdin writer panicked.
const WRITER_PANICKED: &str = "the input writer thread panicked";

/// The error when an output reader panicked.
const READER_PANICKED: &str = "an output reader thread panicked";

/// The threads that feed the child's stdin and drain its stdout and stderr.
struct Workers {
    writer: Worker<()>,
    out: Worker<Vec<u8>>,
    err: Worker<Vec<u8>>,
}

impl Workers {
    /// Takes the child's piped streams, `input`'s copy for its stdin (the writer may outlive
    /// this call, see [`Workers::abandon`], so it cannot borrow), and starts a named thread
    /// for each. Threads already started when one fails end once the caller kills the child.
    fn spawn(child: &mut Child, input: Option<&[u8]>, cap: u64, label: &str) -> io::Result<Self> {
        let stdin = input.and_then(|bytes| Some((child.stdin.take()?, bytes.to_vec())));
        let writer = Worker::spawn(
            format!("{label}-stdin"),
            stdin,
            write_input,
            WRITER_PANICKED,
        )?;
        let read_out = move |pipe| read_capped(pipe, cap);
        let stdout = child.stdout.take();
        let out = Worker::spawn(format!("{label}-stdout"), stdout, read_out, READER_PANICKED)?;
        let read_err = move |pipe| read_capped(pipe, cap);
        let stderr = child.stderr.take();
        let err = Worker::spawn(format!("{label}-stderr"), stderr, read_err, READER_PANICKED)?;
        Ok(Self { writer, out, err })
    }

    fn are_finished(&self) -> bool {
        self.writer.is_finished() && self.out.is_finished() && self.err.is_finished()
    }

    /// Joins every thread into the child's [`Output`].
    fn join(self, status: ExitStatus) -> Result<Output, WaitError> {
        self.writer.join()?;
        Ok(Output {
            status,
            stdout: self.out.join()?,
            stderr: self.err.join()?,
        })
    }

    /// Gives up on the threads, dropping their handles; their results are not wanted. One
    /// still running is blocked on a pipe whose other end a grandchild of the child inherited:
    /// std can neither interrupt a blocked read or write nor kill a process group without
    /// `unsafe`. It holds only that pipe end and at most `cap` bytes of output (or its copy of
    /// the input), and ends when the grandchild closes the pipe or exits, or when this process
    /// exits, whichever comes first (H-13).
    fn abandon(self) {
        drop(self);
    }
}

/// Polls `done` every [`POLL_INTERVAL`] for up to `limit`; whether it became true.
fn poll_until(limit: Duration, done: impl Fn() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if done() {
            return true;
        }
        if started.elapsed() >= limit {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// A named thread that owns one of the child's pipes; none when that stream is not piped.
struct Worker<T> {
    handle: Option<JoinHandle<io::Result<T>>>,
    /// The error's message when the thread panicked.
    panicked: &'static str,
}

impl<T: Default + Send + 'static> Worker<T> {
    fn spawn<P: Send + 'static>(
        name: String,
        pipe: Option<P>,
        work: impl FnOnce(P) -> io::Result<T> + Send + 'static,
        panicked: &'static str,
    ) -> io::Result<Self> {
        let handle = match pipe {
            Some(pipe) => Some(
                thread::Builder::new()
                    .name(name)
                    .spawn(move || work(pipe))?,
            ),
            None => None,
        };
        Ok(Self { handle, panicked })
    }

    fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// The thread's result, or the default when nothing was piped.
    fn join(self) -> Result<T, WaitError> {
        let Some(handle) = self.handle else {
            return Ok(T::default());
        };
        let panicked = self.panicked;
        handle
            .join()
            .map_err(|_payload| WaitError::Io(io::Error::other(panicked)))?
            .map_err(WaitError::Io)
    }
}

/// Writes `input` to the child's stdin, then closes it by dropping the pipe, so the child reads
/// the end of its input.
fn write_input<W: Write>((mut pipe, input): (W, Vec<u8>)) -> io::Result<()> {
    match pipe.write_all(&input) {
        // The child exited, or closed its stdin, before reading it all.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        written => written,
    }
}

/// Reads `pipe` to its end, keeping the first `cap` bytes and discarding the rest, so the
/// writer never blocks on a full pipe and memory stays bounded.
pub(crate) fn read_capped<R: Read>(mut pipe: R, cap: u64) -> io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    pipe.by_ref().take(cap).read_to_end(&mut kept)?;
    io::copy(&mut pipe, &mut io::sink())?;
    Ok(kept)
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
    #[cfg(unix)]
    use std::time::Instant;

    #[cfg(unix)]
    use super::wait_with_deadline;
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

    #[test]
    #[cfg(unix)]
    fn a_child_that_never_reads_and_outlives_the_timeout_is_killed_and_the_writer_joined() {
        // `exec` leaves no shell holding the pipes; 2 MiB is far past any pipe buffer, so the
        // writer blocks until the kill closes the read end.
        let mut child = Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh starts");
        let limits = Limits {
            timeout: Duration::from_millis(200),
            cap: 1024,
        };

        let result = wait_with_input(&mut child, &vec![b'x'; 2 * 1024 * 1024], limits, "sleep");

        assert!(
            matches!(result, Err(super::WaitError::Timeout)),
            "{result:?}"
        );
        assert!(
            child.try_wait().expect("waitable").is_some(),
            "the child was reaped"
        );
    }

    /// The timeout the grandchild tests give their child.
    #[cfg(unix)]
    const SHORT: Duration = Duration::from_millis(200);

    /// Well past [`SHORT`] and its margin, well under the grandchild's 30 s.
    #[cfg(unix)]
    const BOUND: Duration = Duration::from_secs(5);

    /// `sh -c script` with every stream piped. A script whose last command is not a simple
    /// command keeps `sh` from exec'ing it, so its `sleep` is a grandchild holding the pipes.
    #[cfg(unix)]
    fn piped_sh(script: &str) -> std::process::Child {
        Command::new("sh")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh starts")
    }

    #[test]
    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "spawns sh")]
    fn a_grandchild_holding_stdout_does_not_hold_the_wait_past_the_timeout() {
        let mut child = piped_sh("sleep 30; exit 0");

        // The one clock read: the bound is wall time by definition.
        let started = Instant::now();
        let result = wait_with_deadline(&mut child, SHORT, 1024, "sh");
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(super::WaitError::Timeout)),
            "{result:?}"
        );
        assert!(elapsed < BOUND, "returned at the deadline: {elapsed:?}");
        assert!(
            child.try_wait().expect("waitable").is_some(),
            "the child was killed and reaped"
        );
    }

    #[test]
    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "spawns sh")]
    fn a_child_that_exits_leaving_a_grandchild_on_its_pipes_times_out_at_the_deadline() {
        // The child exits at once; the output is not complete while the grandchild can write.
        let mut child = piped_sh("sleep 30 & exit 0");

        // The one clock read: the bound is wall time by definition.
        let started = Instant::now();
        let result = wait_with_deadline(&mut child, SHORT, 1024, "sh");
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(super::WaitError::Timeout)),
            "{result:?}"
        );
        assert!(elapsed < BOUND, "returned at the deadline: {elapsed:?}");
    }

    #[test]
    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "spawns sh")]
    fn a_grandchild_holding_stdin_unread_does_not_hold_the_writer_past_the_timeout() {
        // 2 MiB is far past any pipe buffer: the writer blocks until every reader end closes.
        let mut child = piped_sh("sleep 30; exit 0");
        let limits = Limits {
            timeout: SHORT,
            cap: 1024,
        };

        // The one clock read: the bound is wall time by definition.
        let started = Instant::now();
        let result = wait_with_input(&mut child, &vec![b'x'; 2 * 1024 * 1024], limits, "sh");
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(super::WaitError::Timeout)),
            "{result:?}"
        );
        assert!(elapsed < BOUND, "returned at the deadline: {elapsed:?}");
    }
}
