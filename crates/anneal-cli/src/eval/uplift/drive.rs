//! The Tier 3 run: the with arm, then the baseline, through the [`Evaluator`] port.
//!
//! The first batch runs every case with the full plugin. The baseline follows: by default one
//! batch per skill against the plugin with that skill removed, or, given a baseline plugin
//! (`--baseline`), one batch of every case against it. The with arm gets at most half the cost
//! cap, so the baseline always has the rest; each later batch gets the cap minus what the
//! earlier ones reported. None starts with less than a cent left, and a batch that stops
//! short or fails after the first ends the run with the runs so far.

use std::collections::{BTreeMap, BTreeSet};

use super::Trial;
use super::suites::Case;
use super::tally::{Arms, RUNS};
use crate::error_chain;
use crate::eval::real::result::{ResultError, over_asked};
use crate::eval::real::{RouterError, Stop};

/// The least budget, in US dollars, worth launching a batch for.
const MIN_BUDGET_USD: f64 = 0.01;
/// The most of the cost cap the with arm may spend, so the baseline is never left with
/// nothing to compare against.
const WITH_ARM_SHARE: f64 = 0.5;

/// Runs a batch of cases under `claude plugin eval` and reports each run.
pub(crate) trait Evaluator {
    /// Runs each case of `batch` `batch.runs` times against `batch.arm`'s plugin, spending at
    /// most `batch.budget_usd`.
    ///
    /// # Errors
    /// [`RouterError`] when the batch could not run or its result cannot be read.
    fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError>;
}

/// The plugin a batch runs against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arm<'a> {
    /// The plugin under test, whole.
    With,
    /// The plugin with this skill removed.
    Without(&'a str),
    /// The baseline plugin given with `--baseline`.
    Baseline,
}

/// What the baseline is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Baseline {
    /// Each skill's cases run against the plugin with that skill removed.
    SkillRemoved,
    /// Every case runs against the `--baseline` plugin.
    Plugin,
}

/// One batch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Batch<'a> {
    /// 1 for the first batch.
    pub(crate) index: u32,
    /// The plugin it runs against.
    pub(crate) arm: Arm<'a>,
    /// Its cases.
    pub(crate) cases: Vec<&'a Case>,
    /// Runs per case.
    pub(crate) runs: u32,
    /// The most it may spend, in US dollars.
    pub(crate) budget_usd: f64,
}

/// What one batch returned.
#[derive(Debug)]
pub(crate) struct Ran {
    /// Runs by case name.
    pub(crate) runs: BTreeMap<String, Vec<Trial>>,
    /// Its cost in US dollars, as reported (never negative).
    pub(crate) cost_usd: f64,
    /// True when it was killed before it reported its cost.
    pub(crate) cost_is_lower_bound: bool,
    /// Why it stopped short, if it did.
    pub(crate) stop: Option<Stop>,
    /// Why its result was refused once its cost was read (a case or grader listed twice, or
    /// a grader its case does not have), if it was: its runs are dropped, its cost counts.
    pub(crate) refused: Option<ResultError>,
}

/// How the run went.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drove {
    /// Each case's runs in both arms, by case name.
    pub(crate) runs: BTreeMap<String, Arms>,
    /// What the batches cost, in US dollars, as reported.
    pub(crate) spent_usd: f64,
    /// Batches run.
    pub(crate) batches: u32,
    /// Why the run stopped before every batch ran, if it did.
    pub(crate) stop: Option<Stop>,
    /// True when a batch was killed before it reported its cost.
    pub(crate) cost_is_lower_bound: bool,
}

/// The batches' arms and cases, in order: the with arm first, then the baseline.
fn plan<'a>(cases: &'a [Case], baseline: Baseline) -> Vec<(Arm<'a>, Vec<&'a Case>)> {
    if cases.is_empty() {
        // No batch at all: `claude` would run an empty eval directory.
        return Vec::new();
    }
    let mut batches = vec![(Arm::With, cases.iter().collect())];
    match baseline {
        Baseline::Plugin => batches.push((Arm::Baseline, cases.iter().collect())),
        Baseline::SkillRemoved => {
            let mut by_skill: BTreeMap<&'a str, Vec<&'a Case>> = BTreeMap::new();
            for case in cases {
                by_skill.entry(&case.skill).or_default().push(case);
            }
            batches.extend(
                by_skill
                    .into_iter()
                    .map(|(skill, cases)| (Arm::Without(skill), cases)),
            );
        }
    }
    batches
}

/// Adds the checked result `ran` of `batch` to `drove`, returning why the batch stopped
/// short, if it did.
fn take(drove: &mut Drove, batch: &Batch<'_>, ran: Ran) -> Option<Stop> {
    // `result::check` refuses a negative cost and JSON has no infinity or NaN.
    drove.spent_usd += ran.cost_usd;
    drove.cost_is_lower_bound |= ran.cost_is_lower_bound;
    // A case this batch did not ask for says nothing about the suites, not even when another
    // batch asked for it, so it is left out.
    let asked: BTreeSet<String> = batch.cases.iter().map(|case| case.name()).collect();
    for (name, trials) in ran.runs {
        if !asked.contains(&name) {
            continue;
        }
        if let Some(arms) = drove.runs.get_mut(&name) {
            match batch.arm {
                Arm::With => arms.with.extend(trials),
                Arm::Without(_) | Arm::Baseline => arms.base.extend(trials),
            }
        }
    }
    ran.stop
}

/// What a batch against `arm` may spend of `cap_usd` once `spent_usd` is spent: the with arm
/// at most [`WITH_ARM_SHARE`] of the cap, any other batch what is left.
fn budget(arm: Arm<'_>, cap_usd: f64, spent_usd: f64) -> f64 {
    let left_usd = cap_usd - spent_usd;
    match arm {
        Arm::With => left_usd.min(cap_usd * WITH_ARM_SHARE),
        Arm::Without(_) | Arm::Baseline => left_usd,
    }
}

/// Runs `cases` through `evaluator` in both arms, within `cap_usd`.
///
/// # Errors
/// The first batch's [`RouterError`]: nothing has run, so there is nothing to report. A later
/// batch's failure ends the run with [`Stop::Failed`] instead, keeping the runs so far and
/// marking the cost a lower bound. A batch reporting more runs of a case than it asked for
/// ([`ResultError::TooManyRuns`]), or whose result the evaluator refused once its cost was
/// read ([`Ran::refused`]), is refused the same way, its runs dropped and its cost kept.
pub(crate) fn drive<E: Evaluator>(
    evaluator: &mut E,
    cases: &[Case],
    baseline: Baseline,
    cap_usd: f64,
) -> Result<Drove, RouterError> {
    let mut drove = Drove {
        runs: cases
            .iter()
            .map(|case| (case.name(), Arms::default()))
            .collect(),
        spent_usd: 0.0,
        batches: 0,
        stop: None,
        cost_is_lower_bound: false,
    };
    for (index, (arm, batch_cases)) in (1_u32..).zip(plan(cases, baseline)) {
        let budget_usd = budget(arm, cap_usd, drove.spent_usd);
        if budget_usd < MIN_BUDGET_USD {
            drove.stop = Some(Stop::CostCap);
            break;
        }
        let batch = Batch {
            index,
            arm,
            cases: batch_cases,
            runs: RUNS,
            budget_usd,
        };
        let mut ran = match evaluator.run(&batch) {
            Ok(ran) => ran,
            Err(error) if index == 1 => return Err(error),
            Err(error) => {
                // `claude` may have run and spent before failing; its cost is unknown.
                drove.cost_is_lower_bound = true;
                drove.stop = Some(Stop::Failed {
                    reason: error_chain::reason(&error),
                });
                break;
            }
        };
        if let Some(refused) = ran
            .refused
            .take()
            .or_else(|| over_asked(&ran.runs, batch.runs))
        {
            let error = RouterError::Result(refused);
            if index == 1 {
                return Err(error);
            }
            // The batch reported its cost, so the spend stays exact; its runs are dropped.
            drove.spent_usd += ran.cost_usd;
            drove.batches = index;
            drove.stop = Some(Stop::Failed {
                reason: error_chain::reason(&error),
            });
            break;
        }
        drove.batches = index;
        let stop = take(&mut drove, &batch, ran);
        if stop.is_some() {
            drove.stop = stop;
            break;
        }
    }
    Ok(drove)
}

#[cfg(test)]
mod tests;
