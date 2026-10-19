//! Errors that stop a command before it produces a result, each mapped to a global exit code.
//!
//! A command's own outcomes (a cycle, a red test, findings) are not errors: they are an
//! [`Envelope`](crate::envelope::Envelope) with a per-command code from
//! [`exit_codes`].

use std::io;

use anneal_ledger::{StoreError, TimestampError};

use crate::baseline::BaselineError;
use crate::brief::BriefError;
use crate::brief::ticket::TicketError;
use crate::calibrate::CalibrateError;
use crate::command::CommandName;
use crate::dag::DagError;
use crate::decide::DecideError;
use crate::eval::routing::RoutingError;
use crate::eval::uplift::UpliftError;
use crate::exit_codes::{self, Code};
use crate::floor_guard::FloorError;
use crate::grill::GrillError;
use crate::lint::skills::LintError;
use crate::migrate::MigrateError;
use crate::mode::ModeError;
use crate::paths::PathError;
use crate::perf::PerfError;
use crate::pr_diff::PrError;
use crate::repro::ReproError;
use crate::rollout::RolloutError;
use crate::snapshot::SnapshotError;
use crate::task::TaskError;
use crate::verify::VerifyError;
use crate::watch::WatchError;

/// Why `anneal` could not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The command line was invalid. `reason` is one line.
    #[error("{reason}")]
    Usage {
        /// What was wrong with the arguments.
        reason: String,
    },
    /// The command is in the tree but not built yet.
    #[error("anneal {command} is not implemented yet")]
    NotImplemented {
        /// The stubbed command.
        command: CommandName,
    },
    /// Resolving a path (git common dir, anneal home) failed.
    #[error("resolving a path")]
    Path(#[from] PathError),
    /// Loading or saving a baseline snapshot failed.
    #[error("baseline snapshot")]
    Baseline(#[from] BaselineError),
    /// `anneal lint` could not check its inputs.
    #[error("lint")]
    Lint(#[from] LintError),
    /// `anneal plan` could not read its plan or the spec it links, or `anneal dag check
    /// --plan` could not read its plan.
    #[error("plan")]
    Plan(#[source] LintError),
    /// The ledger could not be opened, written or read.
    #[error(transparent)]
    Ledger(#[from] StoreError),
    /// The system clock could not be read as a timestamp.
    #[error(transparent)]
    Clock(#[from] TimestampError),
    /// The working directory could not be read.
    #[error("reading the working directory")]
    CurrentDir(#[source] io::Error),
    /// `anneal eval` could not read its inputs.
    #[error("eval")]
    Eval(#[from] RoutingError),
    /// `anneal eval uplift` could not read its inputs or run its first batch.
    #[error("eval")]
    Uplift(#[from] UpliftError),
    /// `anneal dag` could not read the tickets or build their graph.
    #[error("dag")]
    Dag(#[from] DagError),
    /// `anneal grill` could not read its design tree.
    #[error(transparent)]
    Grill(#[from] GrillError),
    /// `anneal task done` could not run its test command.
    #[error(transparent)]
    Task(#[from] TaskError),
    /// `anneal mode` could not read its worktree, plan or design tree.
    #[error(transparent)]
    Mode(#[from] ModeError),
    /// `anneal repro` could not find its worktree, run its command or scan for leftovers.
    #[error(transparent)]
    Repro(#[from] ReproError),
    /// `anneal floor-guard` could not check the diff.
    #[error(transparent)]
    FloorGuard(#[from] FloorError),
    /// `anneal verdict` or `anneal ship` could not read the pull request's change.
    #[error(transparent)]
    Pr(#[from] PrError),
    /// `anneal verify risk` could not read the working tree's change.
    #[error(transparent)]
    Verify(#[from] VerifyError),
    /// `anneal decide` could not ask, or record, its question.
    #[error("decide")]
    Decide(#[from] DecideError),
    /// `anneal calibrate` could not read the log or thresholds, or write its proposals.
    #[error("calibrate")]
    Calibrate(#[from] CalibrateError),
    /// `anneal watch` could not read its target.
    #[error("watch")]
    Watch(#[from] WatchError),
    /// `anneal rollout decide` could not read its metrics or bands.
    #[error(transparent)]
    Rollout(#[from] RolloutError),
    /// `anneal perf` could not read its baseline, run its command or save its samples.
    #[error(transparent)]
    Perf(#[from] PerfError),
    /// `anneal migrate` could not read the migration's events.
    #[error(transparent)]
    Migrate(#[from] MigrateError),
    /// `anneal brief build` could not read its range or write its files.
    #[error(transparent)]
    Brief(#[from] BriefError),
    /// `anneal snapshot write` refused its input or could not save the snapshot.
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// `anneal round start` lost every try to other tickets' rounds.
    #[error(
        "round start: other tickets' rounds kept starting during {attempts} tries; run it again"
    )]
    RoundContended {
        /// How many tries were made.
        attempts: usize,
    },
    /// `anneal orchestrate park` lost every try to parks of other items of its ticket.
    #[error(
        "orchestrate park: other parks of the ticket kept landing during {attempts} tries; run it \
         again"
    )]
    ParkContended {
        /// How many tries were made.
        attempts: usize,
    },
    /// The ledger holds more `round_started` events than `anneal round` reads, so a ticket's
    /// rounds cannot be counted whole.
    #[error(
        "round: the ledger holds more than {cap} round_started events, more than are read to \
         count a ticket's rounds"
    )]
    TooManyRounds {
        /// How many events are read.
        cap: usize,
    },
    /// stdout or stderr could not be written.
    #[error("writing output")]
    Output(#[source] io::Error),
}

/// A `--ticket` that is not a plain ticket name is a usage error.
impl From<TicketError> for CliError {
    fn from(error: TicketError) -> Self {
        Self::Usage {
            reason: error.to_string(),
        }
    }
}

impl CliError {
    /// The documented exit code for this error. Never 0.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Usage { .. } => exit_codes::USAGE,
            Self::NotImplemented { .. } => exit_codes::NOT_IMPLEMENTED,
            Self::Path(_)
            | Self::Baseline(_)
            | Self::Ledger(_)
            | Self::Clock(_)
            | Self::CurrentDir(_)
            | Self::Dag(_)
            | Self::RoundContended { .. }
            | Self::ParkContended { .. }
            | Self::TooManyRounds { .. } => exit_codes::INTERNAL,
            Self::Lint(error) | Self::Plan(error) => error.code(),
            Self::Eval(error) => error.code(),
            Self::Uplift(error) => error.code(),
            Self::Grill(error) => error.code(),
            Self::Task(error) => error.code(),
            Self::Mode(error) => error.code(),
            Self::Repro(error) => error.code(),
            Self::FloorGuard(error) => error.code(),
            Self::Pr(error) => error.code(),
            Self::Verify(error) => error.code(),
            Self::Decide(error) => error.code(),
            Self::Calibrate(error) => error.code(),
            Self::Watch(error) => error.code(),
            Self::Rollout(error) => error.code(),
            Self::Perf(error) => error.code(),
            Self::Migrate(error) => error.code(),
            Self::Brief(error) => error.code(),
            Self::Snapshot(error) => error.code(),
            Self::Output(_) => exit_codes::OUTPUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CliError;
    use crate::command::CommandName;
    use crate::paths::PathError;
    use std::io;

    #[test]
    fn each_error_maps_to_its_documented_global_code() {
        let cases = [
            (
                CliError::Usage {
                    reason: "unknown argument: x".to_owned(),
                },
                2,
            ),
            (
                CliError::NotImplemented {
                    command: CommandName::Ledger,
                },
                64,
            ),
            (CliError::Path(PathError::NoHome), 70),
            (CliError::Output(io::Error::other("closed")), 74),
        ];
        for (error, expected) in cases {
            assert_eq!(error.code().value(), expected, "{error:?}");
            assert!(!error.code().is_success(), "an error never exits 0");
        }
    }

    #[test]
    fn not_implemented_names_the_command() {
        let error = CliError::NotImplemented {
            command: CommandName::FloorGuard,
        };
        assert_eq!(
            error.to_string(),
            "anneal floor-guard is not implemented yet",
            "the reason names the command as typed"
        );
    }
}
