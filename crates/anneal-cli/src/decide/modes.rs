//! Each decision point's mode now, as `decide` would run it: the configured mode and the mode
//! in effect. `anneal status` (its `clef` section) and `anneal calibrate` (each point's `mode`)
//! both read it here, so they cannot disagree.

use std::collections::BTreeMap;
use std::ffi::OsString;

use crate::decide::client::effective_mode;
use crate::decide::config::{ClefConfig, ConfigError, Mode};
use crate::decide::log::{DecisionLog, LogError};
use crate::decide::point::Point;

/// The Clef configuration and each point's answered-decision count (`counts.json`).
#[derive(Debug)]
pub(crate) struct Modes {
    config: ClefConfig,
    answered: BTreeMap<Point, u64>,
}

/// Why the modes could not be read. Each variant shows as its source alone, so a reason built
/// from it reads as the configuration's or the log's own.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ModesError {
    /// The configuration does not parse.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The answered-decision counts cannot be read.
    #[error(transparent)]
    Counts(#[from] LogError),
}

impl Modes {
    /// Reads the configuration through `lookup`, then the counts of `log`.
    ///
    /// # Errors
    /// [`ModesError::Config`] for a configuration that does not parse;
    /// [`ModesError::Counts`] when the counts file cannot be read or is not ours.
    pub(crate) fn read(
        log: &DecisionLog,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, ModesError> {
        let config = ClefConfig::from_env(lookup)?;
        let answered = log.answered_by_point()?;
        Ok(Self { config, answered })
    }

    /// Whether an account is configured; without one every point is off.
    pub(crate) fn is_configured(&self) -> bool {
        self.config.account().is_some()
    }

    /// Whether any point has an answered decision.
    pub(crate) fn any_answered(&self) -> bool {
        !self.answered.is_empty()
    }

    /// The answered decisions counted for `point`.
    pub(crate) fn answered(&self, point: Point) -> u64 {
        self.answered.get(&point).copied().unwrap_or(0)
    }

    /// The mode configured for `point`.
    pub(crate) fn configured(&self, point: Point) -> Mode {
        self.config.mode(point)
    }

    /// The mode in effect for `point`: `off` without an account, `shadow` while a configured
    /// `enforce` has fewer than 50 answered decisions, else the configured one.
    pub(crate) fn effective(&self, point: Point) -> Mode {
        if self.is_configured() {
            effective_mode(self.configured(point), self.answered(point))
        } else {
            Mode::Off
        }
    }
}
