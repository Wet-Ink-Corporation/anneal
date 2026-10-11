//! Errors that stop a command before it produces a result, each mapped to a global exit code.
//!
//! A command's own outcomes (a cycle, a red test, findings) are not errors: they are an
//! [`Envelope`](crate::envelope::Envelope) with a per-command code from
//! [`exit_codes`].

use std::io;

use anneal_ledger::{StoreError, TimestampError};

use crate::baseline::BaselineError;
use crate::command::CommandName;
use crate::dag::DagError;
use crate::eval::routing::RoutingError;
use crate::exit_codes::{self, Code};
use crate::grill::GrillError;
use crate::lint::skills::LintError;
use crate::paths::PathError;

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
    /// `anneal plan` could not read its plan or the spec it links.
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
    /// `anneal dag` could not read the tickets or build their graph.
    #[error("dag")]
    Dag(#[from] DagError),
    /// `anneal grill` could not read its design tree.
    #[error(transparent)]
    Grill(#[from] GrillError),
    /// stdout or stderr could not be written.
    #[error("writing output")]
    Output(#[source] io::Error),
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
            | Self::Dag(_) => exit_codes::INTERNAL,
            Self::Lint(error) | Self::Plan(error) => error.code(),
            Self::Eval(error) => error.code(),
            Self::Grill(error) => error.code(),
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
