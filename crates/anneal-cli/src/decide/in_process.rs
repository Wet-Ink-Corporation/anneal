//! `anneal decide` for a caller in this process (a hook): the thresholds, configuration and
//! log the command would use, read the way it reads them, so a hook asks through
//! [`client::ask`] or [`client::ask_recording`] with the same modes, shadow count and rows as
//! the command, without spawning it.
//!
//! The thresholds come from `$CLAUDE_PLUGIN_ROOT/playbooks/clef-thresholds.json`, else the copy
//! built in (a hook has no `--thresholds`); the log is in the anneal home.
//!
//! A hook that must touch nothing while Clef is off asks [`status`] first, which reads only the
//! environment; [`Prepared::label`] appends the outcome row `decide label` would.
//!
//! [`client::ask`]: crate::decide::client::ask
//! [`client::ask_recording`]: crate::decide::client::ask_recording

use std::ffi::OsString;
use std::path::Path;
use std::time::SystemTimeError;

use crate::decide::client::Setup;
use crate::decide::clock::Clock;
use crate::decide::config::{ClefConfig, ConfigError, Mode};
use crate::decide::id::{DecisionId, OutcomeLabel};
use crate::decide::label::{LabelRow, Source};
use crate::decide::log::{DecisionLog, LogError};
use crate::decide::point::Point;
use crate::decide::thresholds::Thresholds;
use crate::decide::{DecideError, read_thresholds, thresholds_path};
use crate::paths::{self, PathError};

/// Whether Clef is on for a point, read from the environment alone.
#[derive(Debug)]
pub(crate) enum Status {
    /// No account, or the point's mode is `off`: ask nothing, log nothing.
    Off,
    /// An account and a mode other than `off`.
    On,
    /// The account, token or mode is malformed.
    Misconfigured(ConfigError),
}

/// Clef's [`Status`] for `point`, from the plugin's `userConfig` in `lookup`; no file is read.
pub(crate) fn status(point: Point, lookup: impl Fn(&str) -> Option<OsString>) -> Status {
    match ClefConfig::from_env(&lookup) {
        Ok(config) if config.account().is_none() || config.mode(point) == Mode::Off => Status::Off,
        Ok(_) => Status::On,
        Err(problem) => Status::Misconfigured(problem),
    }
}

/// Why an outcome row could not be appended.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LabelError {
    /// The system clock is before 1970.
    #[error("reading the clock")]
    Clock(#[source] SystemTimeError),
    /// The row could not be appended.
    #[error(transparent)]
    Log(#[from] LogError),
}

/// What a decision runs with, owned, so [`Prepared::setup`] can lend it to `client::ask`.
#[derive(Debug)]
pub(crate) struct Prepared {
    thresholds: Thresholds,
    config: Result<ClefConfig, ConfigError>,
    log: DecisionLog,
}

/// Why the thresholds or the log could not be found.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrepareError {
    /// The anneal home (for the log) could not be resolved.
    #[error("finding the anneal home")]
    Home(#[source] PathError),
    /// The thresholds file could not be read.
    #[error(transparent)]
    Thresholds(DecideError),
}

impl Prepared {
    /// Reads the thresholds, the configuration and the log's place through `lookup`. A
    /// malformed configuration is kept, not an error: `client::ask` reports it as
    /// `Outcome::Misconfigured`.
    ///
    /// # Errors
    /// [`PrepareError`] when the anneal home cannot be resolved or the plugin's thresholds file
    /// cannot be read or is invalid.
    pub(crate) fn load(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self, PrepareError> {
        let home = paths::anneal_home_from(&lookup).map_err(PrepareError::Home)?;
        // No `--thresholds` here, so the working directory is never read.
        let path = thresholds_path(None, Path::new(""), &lookup);
        let thresholds = read_thresholds(path.as_deref()).map_err(PrepareError::Thresholds)?;
        Ok(Self {
            thresholds,
            config: ClefConfig::from_env(&lookup),
            log: DecisionLog::in_home(&home),
        })
    }

    /// The setup `client::ask` takes, borrowing from `self`.
    pub(crate) fn setup(&self) -> Setup<'_> {
        Setup {
            thresholds: &self.thresholds,
            config: self.config.as_ref(),
            log: &self.log,
        }
    }

    /// Appends the outcome row `anneal decide label <id> <outcome>` would.
    ///
    /// # Errors
    /// [`LabelError::Clock`] for a clock before 1970, [`LabelError::Log`] when the row cannot
    /// be appended.
    pub(crate) fn label(
        &self,
        id: &DecisionId,
        outcome: &OutcomeLabel,
        clock: &impl Clock,
    ) -> Result<(), LabelError> {
        let ts_ms = clock.unix_millis().map_err(LabelError::Clock)?;
        // A hook labels from its own reading: neither a human nor a model is claimed.
        let row = LabelRow::new(id, outcome, ts_ms, Source::default());
        Ok(self.log.append(&row)?)
    }

    /// The decision log's directory, where a hook keeps its own state about its decisions.
    pub(crate) fn log_dir(&self) -> &Path {
        self.log.dir()
    }
}

#[cfg(test)]
mod tests;
