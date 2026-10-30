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
mod fixture;
mod gated;
mod report;
mod stats;
mod suites;
mod tally;
#[cfg(all(test, unix))]
mod test_git;

use std::path::Path;

pub use args::{GatedTools, Scaffolds, UpliftArgs};
pub(crate) use args::{Mode, Settings};
pub use gated::GatedCases;
pub(crate) use report::report;

use crate::eval::real::args::plugin_of;
use crate::eval::real::claude::plugin_name;
use crate::eval::real::{ChangedError, RouterError};
use crate::exit_codes::Code;
use crate::lint::skills::LintError;
use crate::lint::skills::listing::listings;
use case::Grader;
use claude::{BaselinePlugin, ClaudeEvaluator, LauncherTarget, Placement, Plugins};
use drive::{Baseline, Drove, Metered, drive, first_batch_error};
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
    /// Where the plugin copies got the running binary, when a run made copies.
    pub(crate) anneal: Option<PlacedAnneal>,
}

/// Where a run's plugin copies got the running `anneal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlacedAnneal {
    /// Under `libexec/<target>/`, the launcher's first choice on this host.
    Target(&'static str),
    /// Nowhere: the launcher knows no target for this host.
    UnknownHost,
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
    /// A run's selected cases name gated tools and `--allow-gated-tools` was not given.
    #[error(
        "gated tools (Write, Edit or Bash) are named by {0}; review them, then pass \
         --allow-gated-tools to grant them"
    )]
    GatedTools(GatedCases),
    /// A run's selected cases name fixtures and `--allow-scaffolds` was not given.
    #[error(
        "fixtures are named by {0}, whose scaffold scripts run outside the eval sandbox with \
         your rights; review them, then pass --allow-scaffolds to run them"
    )]
    Scaffolds(GatedCases),
}

impl UpliftError {
    /// The documented exit code: an unknown `--skill` is a usage error, as are a `--baseline`
    /// whose manifest cannot be read (a bad argument, not a router failure), gated tools
    /// without `--allow-gated-tools`, fixtures without `--allow-scaffolds` and a missing or
    /// wrong-kind path ([`LintError::code`]); the evaluator's failures are
    /// [`RouterError::code`]'s.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Skills(error) | Self::Suites(error) => error.code(),
            Self::UnknownSkill { .. }
            | Self::Baseline(_)
            | Self::GatedTools(_)
            | Self::Scaffolds(_) => crate::exit_codes::USAGE,
            Self::Router(error) => error.code(),
            Self::Changed(error) => error.code(),
        }
    }
}

/// Reads the suites under `args.suites` for the skills under `args.skills` and, unless
/// `mode` is [`Mode::Check`], runs them through `claude plugin eval`.
///
/// # Errors
/// [`UpliftError`] when a directory cannot be read, `--skill` names a skill with no suites,
/// `--changed-since` cannot compare, a run's selected cases name gated tools without
/// `--allow-gated-tools` or fixtures without `--allow-scaffolds`, the running binary cannot be
/// found, or the evaluator cannot run its first batch.
pub(crate) fn evaluate(args: &UpliftArgs, mode: &Mode<'_>) -> Result<UpliftOutcome, UpliftError> {
    let mut outcome = UpliftOutcome {
        check: matches!(mode, Mode::Check),
        suites: select(args)?,
        drove: None,
        skipped: false,
        baseline: None,
        cleanup: None,
        max_cost_usd: None,
        anneal: None,
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
    }
    // Nothing selected (no change since the base, or no case at all): nothing to run, so
    // `claude` is never started on an empty eval directory.
    if outcome.suites.cases.is_empty() {
        outcome.skipped = true;
        return Ok(outcome);
    }
    // The selection is final: refuse gated tools and scaffolds before anything is copied,
    // spent or run.
    if settings.gated_tools == GatedTools::Refuse
        && let Some(cases) = gated::refusal(&outcome.suites.cases)
    {
        return Err(UpliftError::GatedTools(cases));
    }
    if settings.scaffolds == Scaffolds::Refuse
        && let Some(cases) = gated::scaffold_refusal(&outcome.suites.cases)
    {
        return Err(UpliftError::Scaffolds(cases));
    }
    let target = LauncherTarget::host();
    let (drove, cleanup) = run(settings, &args.skills, &outcome.suites, target)?;
    outcome.drove = Some(drove);
    outcome.cleanup = cleanup;
    outcome.anneal = Some(target.map_or(PlacedAnneal::UnknownHost, |target| {
        PlacedAnneal::Target(target.triple())
    }));
    Ok(outcome)
}

/// Reads the suites under `args.suites` for the skills under `args.skills`, keeping only the
/// `--skill`s' cases and coverage when any is named.
fn select(args: &UpliftArgs) -> Result<Suites, UpliftError> {
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
    Ok(suites)
}

/// Runs `suites`' cases in both arms, each plugin copy holding the running binary under the
/// launcher's `target` when there is one; with the run, why the workspace could not be removed
/// afterwards, if it could not.
fn run(
    settings: &Settings<'_>,
    skills: &Path,
    suites: &Suites,
    target: Option<LauncherTarget>,
) -> Result<(Drove, Option<String>), UpliftError> {
    let binary = target
        .map(|_| std::env::current_exe())
        .transpose()
        .map_err(RouterError::Workspace)?;
    let anneal = binary
        .as_deref()
        .zip(target)
        .map(|(binary, target)| Placement { binary, target });
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
    let mut evaluator = ClaudeEvaluator::new(
        settings.options,
        Plugins {
            dir: plugin_dir,
            skills_dir: settings.skills_dir,
            name: &plugin,
            baseline,
            anneal,
        },
    )?;
    let mut metered = Metered::new(&mut evaluator);
    let drove = drive(&mut metered, &suites.cases, kind, settings.max_cost_usd);
    let reported_usd = metered.reported_usd();
    let finished = evaluator.finish();
    match (drove, finished) {
        (Ok(drove), finished) => Ok((
            drove,
            finished
                .err()
                .map(|error| crate::error_chain::reason(&error)),
        )),
        (Err(error), Ok(())) => Err(UpliftError::Router(first_batch_error(error, reported_usd))),
        (Err(error), Err(removing)) => {
            let error = first_batch_error(error, reported_usd);
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
    /// Whether the run passed its case, whose graders are `graders`: every one of them is
    /// listed as passed (a grader the run leaves out failed), and there is one. `None` when
    /// the run errored.
    pub(crate) fn passed(&self, graders: &[Grader]) -> Option<bool> {
        match self {
            Self::Errored => None,
            Self::Graded(_) => Some(
                !graders.is_empty()
                    && graders
                        .iter()
                        .all(|grader| self.grader(grader.name()) == Some(true)),
            ),
        }
    }

    /// Whether the run errored, so is no trial.
    pub(crate) const fn errored(&self) -> bool {
        matches!(self, Self::Errored)
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
