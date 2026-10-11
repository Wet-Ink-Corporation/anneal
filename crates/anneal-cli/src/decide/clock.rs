//! The clocks a decision reads: the wall clock for ids and rows, a monotonic one for latency.
//! A port, so tests use fixed times (TST-10).

use std::time::{Duration, Instant, SystemTime, SystemTimeError, UNIX_EPOCH};

/// The wall clock for ids and rows, and a monotonic one for latency.
pub trait Clock {
    /// Milliseconds since the Unix epoch.
    ///
    /// # Errors
    /// [`SystemTimeError`] for a clock set before 1970.
    fn unix_millis(&self) -> Result<u64, SystemTimeError>;

    /// Time since an arbitrary fixed start, never going back.
    fn monotonic(&self) -> Duration;
}

/// The system's clocks.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Clocks whose monotonic reading starts now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        let since = SystemTime::now().duration_since(UNIX_EPOCH)?;
        Ok(u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
    }

    fn monotonic(&self) -> Duration {
        self.start.elapsed()
    }
}
