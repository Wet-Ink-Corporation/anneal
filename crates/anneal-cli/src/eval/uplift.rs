//! `anneal eval uplift`: Tier 3, behaviour uplift (`evals/README.md`).
//!
//! Each entry skill has suites of cases under `evals/uplift/<skill>/{regression,capability}/`
//! (`suites`, `case`). Every problem with them stops the run before anything is spent. The
//! cases then run three times each through `claude plugin eval` (the `drive::Evaluator`
//! port, adapted by `claude`), once against the whole plugin and once against the baseline:
//! the plugin with the case's skill removed, or the `--baseline` plugin. Each skill is held to
//! the bars (`tally`, `stats`): a pooled uplift whose 95% Newcombe lower bound is 0 or more,
//! every guard case passing all three runs with the skill (pass^3), and no regression suite
//! regressed.

mod args;
mod case;
mod changed;
mod claude;
mod drive;
mod report;
mod stats;
mod suites;
mod tally;

use std::path::Path;

pub use args::UpliftArgs;
pub(crate) use args::{Mode, Settings};
pub(crate) use report::report;

use crate::eval::routing::real::args::plugin_of;
use crate::eval::routing::real::claude::plugin_name;
use crate::eval::routing::real::{ChangedError, RouterError, spend_unknown};
use crate::exit_codes::Code;
use crate::lint::skills::LintError;
use crate::lint::skills::listing::listings;
use claude::{BaselinePlugin, ClaudeEvaluator};
use drive::{Baseline, Drove, drive};
use suites::Suites;

/// What a Tier 3 run found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UpliftOutcome {
    /// True for `--check`: the suites were read and nothing ran.
    pub(crate) check: bool,
    /// The suites: the cases selected (by `--skill` and `--changed-since`), every problem,
    /// and the coverage of the skills selected.
    pub(crate) suites: Suites,
    /// How the batches went, or `None` when nothing ran.
    pub(crate) drove: Option<Drove>,
    /// True when `--changed-since` found no selected skill changed, so nothing ran.
    pub(crate) skipped: bool,
    /// The baseline, when a run was asked for.
    pub(crate) baseline: Option<BaselineKind>,
    /// Why the temporary workspace could not be removed, if it could not.
    pub(crate) cleanup: Option<String>,
    /// The cost cap in US dollars, when a run was asked for.
    pub(crate) max_cost_usd: Option<f64>,
}

/// The baseline a run compares with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BaselineKind {
    /// Each skill's cases against the plugin with that skill removed.
    SkillRemoved,
    /// Every case against the `--baseline` plugin at this path.
    Plugin(String),
}

/// Why `eval uplift` could not run.
#[derive(Debug, thiserror::Error)]
pub enum UpliftError {
    /// The skills directory could not be read.
    #[error("reading skills")]
    Skills(#[source] LintError),
    /// The suites directory could not be read.
    #[error("reading suites")]
    Suites(#[source] LintError),
    /// `--skill` named a skill with no suite directory.
    #[error("--skill {name} names no skill with suites under the suites directory")]
    UnknownSkill {
        /// The name given.
        name: String,
    },
    /// The `--baseline` plugin's manifest could not be read.
    #[error("reading the --baseline plugin")]
    Baseline(#[source] RouterError),
    /// The evaluator could not run its first batch.
    #[error(transparent)]
    Router(#[from] RouterError),
    /// `--changed-since` could not compare.
    #[error("--changed-since")]
    Changed(#[source] ChangedError),
}

impl UpliftError {
    /// The documented exit code: an unknown `--skill` is a usage error, as is a missing or
    /// wrong-kind path ([`LintError::code`]); the evaluator's failures are
    /// [`RouterError::code`]'s.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Skills(error) | Self::Suites(error) => error.code(),
            Self::UnknownSkill { .. } => crate::exit_codes::USAGE,
            Self::Baseline(error) | Self::Router(error) => error.code(),
            Self::Changed(error) => error.code(),
        }
    }
}

/// Reads the suites under `args.suites` for the skills under `args.skills` and, unless
/// `mode` is [`Mode::Check`], runs them through `claude plugin eval`.
///
/// # Errors
/// [`UpliftError`] when a directory cannot be read, `--skill` names a skill with no suites,
/// `--changed-since` cannot compare, or the evaluator cannot run its first batch.
pub(crate) fn evaluate(args: &UpliftArgs, mode: &Mode<'_>) -> Result<UpliftOutcome, UpliftError> {
    let skills: Vec<String> = listings(std::slice::from_ref(&args.skills))
        .map_err(UpliftError::Skills)?
        .into_iter()
        .map(|listed| match listed {
            Ok(listing) => listing.name,
            Err(unparsed) => unparsed.name,
        })
        .collect();
    let mut suites = suites::read(&args.suites, &skills).map_err(UpliftError::Suites)?;
    if let Some(name) = args
        .skill
        .iter()
        .find(|name| !suites.coverage.iter().any(|row| &row.skill == *name))
    {
        return Err(UpliftError::UnknownSkill {
            name: name.to_owned(),
        });
    }
    if !args.skill.is_empty() {
        suites.cases.retain(|case| args.skill.contains(&case.skill));
        suites
            .coverage
            .retain(|row| args.skill.contains(&row.skill));
    }
    let mut outcome = UpliftOutcome {
        check: matches!(mode, Mode::Check),
        suites,
        drove: None,
        skipped: false,
        baseline: None,
        cleanup: None,
        max_cost_usd: None,
    };
    let Mode::Run(settings) = mode else {
        return Ok(outcome);
    };
    outcome.max_cost_usd = Some(settings.max_cost_usd);
    outcome.baseline = Some(settings.baseline.map_or(BaselineKind::SkillRemoved, |dir| {
        BaselineKind::Plugin(dir.display().to_string())
    }));
    if !outcome.suites.problems.is_empty() {
        return Ok(outcome);
    }
    if let Some(base) = settings.changed_since {
        let changed = changed::changed_skills(&args.skills, &args.suites, base)
            .map_err(UpliftError::Changed)?;
        outcome
            .suites
            .cases
            .retain(|case| changed.contains(&case.skill));
        if outcome.suites.cases.is_empty() {
            outcome.skipped = true;
            return Ok(outcome);
        }
    }
    let (drove, cleanup) = run(settings, &args.skills, &outcome.suites)?;
    outcome.drove = Some(drove);
    outcome.cleanup = cleanup;
    Ok(outcome)
}

/// Runs `suites`' cases in both arms; with the run, why the workspace could not be removed
/// afterwards, if it could not.
fn run(
    settings: &Settings<'_>,
    skills: &Path,
    suites: &Suites,
) -> Result<(Drove, Option<String>), UpliftError> {
    let plugin_dir = plugin_of(skills);
    let plugin = plugin_name(plugin_dir)?;
    let baseline_name = settings
        .baseline
        .map(plugin_name)
        .transpose()
        .map_err(UpliftError::Baseline)?;
    let baseline = settings
        .baseline
        .zip(baseline_name.as_deref())
        .map(|(dir, name)| BaselinePlugin { dir, name });
    let kind = if baseline.is_some() {
        Baseline::Plugin
    } else {
        Baseline::SkillRemoved
    };
    let mut evaluator = ClaudeEvaluator::new(settings.options, plugin_dir, &plugin, baseline)?;
    let drove = drive(&mut evaluator, &suites.cases, kind, settings.max_cost_usd);
    let finished = evaluator.finish();
    match (drove, finished) {
        (Ok(drove), finished) => Ok((
            drove,
            finished
                .err()
                .map(|error| crate::error_chain::reason(&error)),
        )),
        (Err(error), Ok(())) => Err(UpliftError::Router(spend_unknown(error))),
        (Err(error), Err(removing)) => {
            let error = spend_unknown(error);
            Err(UpliftError::Router(RouterError::Cleanup {
                code: error.code(),
                error: Box::new(error),
                removing: Box::new(removing),
            }))
        }
    }
}

/// What one run of a case showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Trial {
    /// The run ended in an error other than running out of turns (a timeout, a crash): no
    /// evidence either way, so not a trial.
    Errored,
    /// The run finished, or ran out of turns, and was graded: each grader's name and whether
    /// it passed.
    Graded(Vec<(String, bool)>),
}

impl Trial {
    /// Whether the run passed (every grader passed, and there was one), or `None` when it
    /// errored.
    pub(crate) fn passed(&self) -> Option<bool> {
        match self {
            Self::Errored => None,
            Self::Graded(graders) => {
                Some(!graders.is_empty() && graders.iter().all(|(_, passed)| *passed))
            }
        }
    }

    /// Whether the grader named `name` passed, or `None` when the run errored. A grader the
    /// run does not list did not pass.
    pub(crate) fn grader(&self, name: &str) -> Option<bool> {
        match self {
            Self::Errored => None,
            Self::Graded(graders) => Some(
                graders
                    .iter()
                    .any(|(grader, passed)| grader == name && *passed),
            ),
        }
    }
}
