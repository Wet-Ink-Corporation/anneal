//! `anneal eval routing --router claude`: Tier 2, the real router (`evals/README.md`).
//!
//! The Tier 1 fixtures are read and checked as Tier 1 reads them; any problem stops the run
//! before anything is spent. Each positive prompt then runs under Claude Code's router through
//! `claude plugin eval` (the `drive::Router` port, adapted by `claude`), in batches of three
//! until its should-fire interval settles (`schedule`) or the cost cap or time limit is
//! reached. Each skill is then held to the bars (`tally`, `wilson`): every one of its own
//! prompts settles with a 95% Wilson lower bound of at least 0.70, and the upper bound pooled
//! over its siblings' prompts is at most 0.20.

mod args;
mod changed;
mod claude;
mod drive;
mod report;
mod result;
mod schedule;
mod suite;
mod tally;
mod wilson;
mod workspace;

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use serde::Serialize;

pub(crate) use args::Settings;
pub use args::{RealArgs, RouterKind};
pub use changed::ChangedError;
pub(crate) use report::report;
pub use result::ResultError;

use crate::eval::routing::{CaseKind, Problem, RoutingArgs, RoutingError, evaluate as read_inputs};
use crate::exit_codes::{self, Code};
use crate::json_file::JsonFileError;
use claude::ClaudeRouter;
use drive::{Drove, drive};
use suite::Graders;
use tally::Probe;

/// What a Tier 2 run found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RealOutcome {
    /// The model-invoked skills, sorted.
    pub(crate) skills: Vec<String>,
    /// Every positive prompt and its runs, by fixture then line.
    pub(crate) probes: Vec<Probe>,
    /// Every problem with the fixtures or skills; any one stops the run before it starts.
    pub(crate) problems: Vec<Problem>,
    /// How the batches went, or `None` when nothing ran.
    pub(crate) drove: Option<Drove>,
    /// True when `--changed-since` found no routing change, so nothing ran.
    pub(crate) skipped: bool,
    /// Why the temporary workspace could not be removed, if it could not.
    pub(crate) cleanup: Option<String>,
    /// The cost cap in US dollars.
    pub(crate) max_cost_usd: f64,
}

/// Runs the positives of the fixtures under `inputs.fixtures` through `claude plugin eval`
/// against the plugin whose skills are `inputs.skills`.
///
/// # Errors
/// [`RoutingError`] when an input directory cannot be read, `--changed-since` cannot be
/// compared, or the router cannot run its first batch.
pub(crate) fn evaluate(
    inputs: &RoutingArgs,
    settings: &Settings<'_>,
) -> Result<RealOutcome, RoutingError> {
    let tier1 = read_inputs(inputs)?;
    let mut outcome = RealOutcome {
        probes: tier1
            .cases
            .into_iter()
            .filter(|case| case.kind == CaseKind::Positive)
            .map(|case| Probe {
                skill: case.expected,
                file: case.file,
                line: case.line,
                text: case.prompt,
                runs: Vec::new(),
            })
            .collect(),
        skills: tier1.skills,
        problems: tier1.problems,
        drove: None,
        skipped: false,
        cleanup: None,
        max_cost_usd: settings.max_cost_usd,
    };
    if !outcome.problems.is_empty() {
        return Ok(outcome);
    }
    if let Some(base) = settings.changed_since
        && !changed::changed(&inputs.skills, base).map_err(RoutingError::Changed)?
    {
        outcome.skipped = true;
        return Ok(outcome);
    }
    let plugin_dir = args::plugin_of(&inputs.skills);
    let plugin = claude::plugin_name(plugin_dir).map_err(RoutingError::Router)?;
    let graders = Graders::new(&plugin, &outcome.skills);
    let mut router = ClaudeRouter::new(settings.options, plugin_dir, &plugin, &graders)
        .map_err(RoutingError::Router)?;
    let drove = drive(&mut router, &mut outcome.probes, settings.max_cost_usd);
    let finished = router.finish();
    match (drove, finished) {
        (Ok(drove), finished) => {
            outcome.drove = Some(drove);
            outcome.cleanup = finished
                .err()
                .map(|error| crate::error_chain::reason(&error));
            Ok(outcome)
        }
        (Err(error), Ok(())) => Err(RoutingError::Router(spend_unknown(error))),
        (Err(error), Err(removing)) => {
            let error = spend_unknown(error);
            Err(RoutingError::Router(RouterError::Cleanup {
                code: error.code(),
                error: Box::new(error),
                removing: Box::new(removing),
            }))
        }
    }
}

/// `error` from the first batch, marked [`RouterError::SpendUnknown`] when `claude` had
/// started and so may have spent before it failed.
fn spend_unknown(error: RouterError) -> RouterError {
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
            | Self::Result(_) => exit_codes::EVAL_ROUTER_FAILED,
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

/// What one batch returned: each case's runs by case name, what it cost, and why it stopped
/// short, if it did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ran {
    /// Runs by case name.
    pub(crate) runs: BTreeMap<String, Vec<Run>>,
    /// The batch's cost in US dollars, as the router reported it (never negative).
    pub(crate) cost_usd: f64,
    /// True when the batch was killed before it reported its cost, so `cost_usd` (then 0)
    /// is less than it spent.
    pub(crate) cost_is_lower_bound: bool,
    /// Runs that ended in error and still count as trials (see `result`).
    pub(crate) counted_errors: u32,
    /// Why the batch stopped short, if it did.
    pub(crate) stop: Option<Stop>,
}

/// What one run of a prompt under the real router showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Run {
    /// The run ended in an error before routing was decided (a timeout or a crash with no
    /// skill fired): no evidence either way.
    Errored,
    /// The run decided routing: it finished, ran out of turns, or fired a skill before its
    /// error.
    Done {
        /// The skills it invoked, by name.
        fired: Vec<String>,
        /// Whether its outcome grader passed: it gave a non-empty reply.
        answered: bool,
    },
}

impl Run {
    /// Whether `skill` fired in this run, or `None` when the run errored.
    pub(crate) fn fired(&self, skill: &str) -> Option<bool> {
        match self {
            Self::Errored => None,
            Self::Done { fired, .. } => Some(fired.iter().any(|name| name == skill)),
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
