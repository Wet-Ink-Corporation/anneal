//! `anneal decide` for a caller in this process (a hook): the thresholds, configuration and
//! log the command would use, read the way it reads them, so a hook asks through
//! [`client::ask`] or [`client::ask_recording`] with the same modes, shadow count and rows as
//! the command, without spawning it.
//!
//! The thresholds come from `$CLAUDE_PLUGIN_ROOT/playbooks/clef-thresholds.json`, else the copy
//! built in (a hook has no `--thresholds`); the log is in the anneal home.
//!
//! [`client::ask`]: crate::decide::client::ask
//! [`client::ask_recording`]: crate::decide::client::ask_recording

use std::ffi::OsString;
use std::path::Path;

use crate::decide::client::Setup;
use crate::decide::config::{ClefConfig, ConfigError};
use crate::decide::log::DecisionLog;
use crate::decide::thresholds::Thresholds;
use crate::decide::{DecideError, read_thresholds, thresholds_path};
use crate::paths::{self, PathError};

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
}
