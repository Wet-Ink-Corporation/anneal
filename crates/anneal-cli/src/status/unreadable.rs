//! Why a section of `anneal status` could not be read: the error of the module that owns the
//! section's data, kept typed until the finding is written (ERR-01).

use anneal_ledger::StoreError;

use crate::context::sessions::SessionsError;
use crate::decide::config::ConfigError;
use crate::decide::log::LogError;
use crate::decide::modes::ModesError;
use crate::error::CliError;
use crate::error_chain::reason;
use crate::lint::skills::LintError;
use crate::pr_diff::DiffError;
use crate::repo_config::GateConfigError;
use crate::verdict::pull_request::ReadError;

/// Why a section could not be read. Each variant carries its source's own error, so the
/// finding's reason is the one-line [`reason`] of it.
#[derive(Debug, thiserror::Error)]
pub(super) enum Unreadable {
    /// The ledger could not be read: `gates`, `ledger_tail`, the mode and the verdicts.
    #[error(transparent)]
    Ledger(#[from] StoreError),
    /// The build plan's file could not be read.
    #[error("the plan {plan}")]
    Plan {
        /// The plan, relative to the worktree root, as build mode names it.
        plan: String,
        #[source]
        source: LintError,
    },
    /// The plan's approvals could not be read (`plan::approval_record`).
    #[error(transparent)]
    Approval(#[from] CliError),
    /// The done tasks could not be read; `hook::handoff::facts::done_since` words it.
    #[error("{0}")]
    DoneTasks(String),
    /// Git could not resolve `HEAD`, or the git budget ran out.
    #[error(transparent)]
    Git(#[from] DiffError),
    /// The repository's gate configuration could not be read.
    #[error(transparent)]
    GateConfig(#[from] GateConfigError),
    /// The pull request's verdicts could not be judged.
    #[error(transparent)]
    PullRequest(#[from] ReadError),
    /// Clef's configuration could not be read from the environment.
    #[error(transparent)]
    ClefConfig(#[from] ConfigError),
    /// The answered-decision counts could not be read.
    #[error(transparent)]
    DecisionLog(#[from] LogError),
    /// The saved context-usage snapshots could not be read.
    #[error(transparent)]
    Sessions(#[from] SessionsError),
}

impl Unreadable {
    /// The reason a `status/unreadable:<section>` finding gives: one line, the error and its
    /// sources, with the plan's path as build mode names it.
    pub(super) fn reason(&self) -> String {
        match self {
            Self::Plan { plan, source } => format!("the plan {plan}: {}", reason(source)),
            other => reason(other),
        }
    }
}

/// The Clef modes fail as the configuration or the counts they read, so `clef`'s finding is
/// the same either way.
impl From<ModesError> for Unreadable {
    fn from(error: ModesError) -> Self {
        match error {
            ModesError::Config(error) => Self::ClefConfig(error),
            ModesError::Counts(error) => Self::DecisionLog(error),
        }
    }
}
