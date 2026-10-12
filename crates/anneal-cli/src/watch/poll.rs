//! The long poll: ask a [`Source`] until it settles or the timeout passes, backing off between
//! polls ([`FIRST_DELAY`], doubling, at most [`MAX_DELAY`]), never sleeping past the deadline.
//!
//! Time is a port ([`Clock`]) so tests never sleep (TST-10). The real [`SystemClock`] sleeps
//! the thread: `watch` is a one-shot command with nothing else to do meanwhile, and Ctrl-C
//! (SIGINT) ends the process wherever it is, a sleep included.

use std::thread;
use std::time::{Duration, Instant};

use super::WatchError;
use super::snapshot::Snapshot;

/// The default `--timeout`, in seconds: 30 minutes.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 30 * 60;
/// The wait after the first poll that finds work pending.
pub(crate) const FIRST_DELAY: Duration = Duration::from_secs(10);
/// The longest wait between two polls.
pub(crate) const MAX_DELAY: Duration = Duration::from_secs(60);

/// Time as the watch sees it.
pub(crate) trait Clock {
    /// How long ago the watch started.
    fn elapsed(&self) -> Duration;
    /// Waits `delay`.
    fn sleep(&self, delay: Duration);
}

/// The real clock: the time since it was started, and a thread sleep.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SystemClock {
    started: Instant,
}

impl SystemClock {
    /// A clock started now.
    pub(crate) fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn sleep(&self, delay: Duration) {
        thread::sleep(delay);
    }
}

/// What the watch polls.
pub(crate) trait Source {
    /// One look at the target.
    ///
    /// # Errors
    /// The [`WatchError`] when the target cannot be read.
    fn poll(&self) -> Result<Snapshot, WatchError>;
}

/// When to give up, and how to back off until then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Schedule {
    /// How long to keep polling.
    pub(crate) timeout: Duration,
    /// The first wait.
    pub(crate) first: Duration,
    /// The longest wait.
    pub(crate) cap: Duration,
}

impl Schedule {
    /// The default back-off with `timeout`.
    pub(crate) const fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            first: FIRST_DELAY,
            cap: MAX_DELAY,
        }
    }
}

/// How the watch ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Watched {
    /// The last poll's snapshot.
    pub(crate) snapshot: Snapshot,
    /// How many polls were made.
    pub(crate) polls: u32,
    /// How long the watch took.
    pub(crate) waited: Duration,
    /// True when work was still pending at the deadline.
    pub(crate) timed_out: bool,
}

/// Polls `source` until a snapshot settles or `schedule.timeout` has passed, sleeping on
/// `clock` between polls. A zero timeout polls once.
///
/// # Errors
/// The first [`WatchError`] a poll returns.
pub(crate) fn watch<S: Source, C: Clock>(
    source: &S,
    clock: &C,
    schedule: Schedule,
) -> Result<Watched, WatchError> {
    let mut delay = schedule.first;
    let mut polls: u32 = 0;
    loop {
        let snapshot = source.poll()?;
        polls = polls.saturating_add(1);
        let left = schedule.timeout.saturating_sub(clock.elapsed());
        if snapshot.settled() || left.is_zero() {
            return Ok(Watched {
                timed_out: !snapshot.settled(),
                snapshot,
                polls,
                waited: clock.elapsed(),
            });
        }
        clock.sleep(delay.min(left));
        delay = delay.saturating_mul(2).min(schedule.cap);
    }
}

#[cfg(test)]
mod tests;
