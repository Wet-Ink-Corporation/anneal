//! `anneal eval routing --router claude`: Tier 2, the real router (`evals/README.md`).
//!
//! The Tier 1 fixtures are read and checked as Tier 1 reads them; any problem stops the run
//! before anything is spent. Each positive prompt then runs under Claude Code's router through
//! `claude plugin eval` (the `drive::Router` port, adapted by `claude`), in batches of three
//! until its should-fire interval settles (`schedule`) or the cost cap or time limit is
//! reached. Each skill is then held to the bars (`tally`, `eval::real::wilson`): every one of
//! its own prompts settles with a 95% Wilson lower bound of at least 0.70, and the upper bound
//! pooled over its siblings' prompts is at most 0.20.
//!
//! The `claude plugin eval` driver, the result header checks, the intervals, the workspace and
//! the defaults are Tier 3's too, so they live in `eval::real`.

mod args;
mod changed;
mod claude;
mod drive;
mod report;
mod result;
mod schedule;
mod suite;
mod tally;

use std::collections::BTreeMap;

pub(crate) use args::Settings;
pub use args::{RealArgs, RouterKind};
pub(crate) use report::report;

use crate::eval::real::args::plugin_of;
use crate::eval::real::claude::plugin_name;
pub use crate::eval::real::{ChangedError, ResultError, RouterError};
pub(crate) use crate::eval::real::{Stop, spend_unknown};
use crate::eval::routing::{CaseKind, Problem, RoutingArgs, RoutingError, evaluate as read_inputs};
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
    let plugin_dir = plugin_of(&inputs.skills);
    let plugin = plugin_name(plugin_dir).map_err(RoutingError::Router)?;
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
