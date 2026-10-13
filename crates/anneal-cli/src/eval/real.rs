//! What Tier 2 (`eval::routing::real`) and Tier 3 (`eval::uplift`) share to run a plugin
//! under `claude plugin eval`: the flags' defaults (`args`), the driver (`claude`), the result
//! reader (`result`), the Wilson interval (`wilson`) and the temporary workspace
//! (`workspace`), with the errors and stop reasons both tiers report.

pub(crate) mod args;
pub(crate) mod claude;
pub(crate) mod result;
pub(crate) mod wilson;
pub(crate) mod workspace;

use std::io;
use std::path::PathBuf;

use serde::Serialize;

pub use result::ResultError;

use crate::exit_codes::{self, Code};
use crate::json_file::JsonFileError;
use crate::lint::skills::LintError;
use crate::paths::PathError;

/// `error` from the first batch, marked [`RouterError::SpendUnknown`] when `claude` had
/// started and so may have spent before it failed.
pub(crate) fn spend_unknown(error: RouterError) -> RouterError {
    match error {
        RouterError::Wait(_)
        | RouterError::Failed { .. }
        | RouterError::Unreadable(_)
        | RouterError::Result(_) => RouterError::SpendUnknown {
            code: error.code(),
            error: Box::new(error),
        },
        RouterError::Spawn { .. }
        | RouterError::Workspace(_)
        | RouterError::Manifest(_)
        | RouterError::Refused { .. }
        | RouterError::SpendUnknown { .. }
        | RouterError::Cleanup { .. } => error,
    }
}

/// Why the real router could not run a batch.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    /// `claude` could not be started.
    #[error("starting {}; install Claude Code 2.1.295 or later, or pass --claude", program.display())]
    Spawn {
        /// The program run.
        program: PathBuf,
        /// Why it did not start.
        #[source]
        source: io::Error,
    },
    /// The temporary eval workspace could not be prepared, read or removed.
    #[error("preparing the eval workspace")]
    Workspace(#[source] io::Error),
    /// `claude plugin eval` could not be waited on or killed.
    #[error("waiting for claude plugin eval")]
    Wait(#[source] io::Error),
    /// The plugin's manifest could not be read for its name.
    #[error("reading the plugin manifest")]
    Manifest(#[source] JsonFileError),
    /// `claude plugin eval` ended without writing a result.
    #[error("claude plugin eval ended ({status}) without a result: {stderr}")]
    Failed {
        /// How it ended.
        status: String,
        /// The end of its stderr, one line.
        stderr: String,
    },
    /// Its result file could not be read or parsed.
    #[error("reading claude plugin eval's result")]
    Unreadable(#[source] JsonFileError),
    /// Its result says nothing about routing.
    #[error(transparent)]
    Result(#[from] ResultError),
    /// The first batch's result was refused once its cost had been read, so what it spent is
    /// known.
    #[error("claude plugin eval reported spending {cost_usd} USD, then its result was refused")]
    Refused {
        /// What the result reported the batch cost, in US dollars.
        cost_usd: f64,
        /// Why it was refused.
        #[source]
        error: ResultError,
    },
    /// The first batch failed after `claude` had started, so it may have spent money that no
    /// result reported.
    #[error("claude plugin eval ran, so its spend is unknown")]
    SpendUnknown {
        /// The failure.
        #[source]
        error: Box<RouterError>,
        /// Its exit code, which the mark keeps.
        code: Code,
    },
    /// The router failed, and removing the workspace afterwards failed too.
    #[error("{error}; removing the eval workspace also failed")]
    Cleanup {
        /// The router's failure.
        error: Box<RouterError>,
        /// Its exit code, which the cleanup failure keeps.
        code: Code,
        /// Why the workspace was not removed.
        #[source]
        removing: Box<RouterError>,
    },
}

impl RouterError {
    /// The documented exit code: the workspace is anneal's own I/O (internal); the rest mean
    /// the router could not run. A failed cleanup keeps the router failure's code.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Workspace(_) => exit_codes::INTERNAL,
            Self::Cleanup { code, .. } | Self::SpendUnknown { code, .. } => *code,
            Self::Spawn { .. }
            | Self::Wait(_)
            | Self::Manifest(_)
            | Self::Failed { .. }
            | Self::Unreadable(_)
            | Self::Result(_)
            | Self::Refused { .. } => exit_codes::EVAL_ROUTER_FAILED,
        }
    }
}

/// Why a batch, and so the whole run, stopped before every prompt settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Stop {
    /// The cost cap was reached.
    CostCap,
    /// The time limit was reached and the router was stopped.
    TimeLimit,
    /// The router reported its run as interrupted.
    Interrupted,
    /// A batch after the first failed; the earlier batches' runs stand.
    Failed {
        /// Why, one line.
        reason: String,
    },
}

/// Why `--changed-since` could not compare.
#[derive(Debug, thiserror::Error)]
pub enum ChangedError {
    /// The base names no commit.
    #[error("--changed-since {base:?} names no commit")]
    Unresolved {
        /// The base as given.
        base: String,
    },
    /// git could not be run.
    #[error("comparing skills with git")]
    Git(#[source] PathError),
    /// The skills directory could not be listed.
    #[error("listing the skills")]
    List(#[source] LintError),
    /// A skill file could not be read.
    #[error("reading {}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: io::Error,
    },
}

impl ChangedError {
    /// The documented exit code: a base that names no commit is a usage error, the rest
    /// internal.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Unresolved { .. } => exit_codes::USAGE,
            Self::Git(_) | Self::List(_) | Self::Read { .. } => exit_codes::INTERNAL,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::RouterError;
    use crate::exit_codes;

    #[test]
    fn a_failure_to_wait_for_claude_is_a_router_failure_not_a_workspace_one() {
        let error = RouterError::Wait(io::Error::other("interrupted"));

        assert_eq!(
            (error.code(), error.to_string()),
            (
                exit_codes::EVAL_ROUTER_FAILED,
                "waiting for claude plugin eval".to_owned()
            )
        );
    }

    #[test]
    fn a_first_batch_that_failed_after_claude_started_says_its_spend_is_unknown() {
        let started = RouterError::Failed {
            status: "exit status: 1".to_owned(),
            stderr: "Error: not logged in".to_owned(),
        };
        let not_started = RouterError::Spawn {
            program: "claude".into(),
            source: io::Error::from(io::ErrorKind::NotFound),
        };

        let started = super::spend_unknown(started);
        let not_started = super::spend_unknown(not_started);

        let reason = crate::error_chain::reason(&started);
        assert!(
            reason.contains("spend is unknown") && reason.contains("Error: not logged in"),
            "{reason}"
        );
        assert_eq!(started.code(), exit_codes::EVAL_ROUTER_FAILED);
        assert!(
            !crate::error_chain::reason(&not_started).contains("spend is unknown"),
            "nothing ran, so nothing was spent"
        );
    }
}
