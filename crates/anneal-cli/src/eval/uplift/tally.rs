//! Scoring a Tier 3 run: pure, no I/O.
//!
//! Each case's runs are counted in both arms (a run passes when every grader of its case is
//! listed as passed; an errored run is no trial). A skill is then held to three bars:
//! - its uplift, pooled over all its cases' runs, has a Newcombe lower bound of 0 or more;
//! - each of its guard cases held pass^3: all three runs with the skill passed;
//! - no regression suite of it regressed: the 95% Wilson upper bound of its pass rate with
//!   the skill is 0.90 or more.
//!
//! Assertions passing in 90% or more of their baseline runs are listed as non-discriminating,
//! and capability suites at 100% as graduating; neither fails the run.

use std::collections::BTreeMap;

use serde::Serialize;

use super::Trial;
use super::case::{Behaviour, Pressure};
use super::stats::{Count, Difference, newcombe};
use super::suites::{Case, SuiteKind};
use crate::eval::real::wilson::wilson;

/// Runs per case in each arm.
pub(crate) const RUNS: u32 = 3;
/// The least Wilson upper bound a regression suite's pass rate with the skill may have.
const REGRESSION_MIN_UPPER: f64 = 0.90;
/// The least lower bound of a skill's uplift.
const UPLIFT_MIN_LOWER: f64 = 0.0;

/// One case's runs in each arm.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Arms {
    /// With the skill.
    pub(crate) with: Vec<Trial>,
    /// Without it: the baseline.
    pub(crate) base: Vec<Trial>,
}

/// One case's result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct CaseResult<'a> {
    pub(crate) skill: &'a str,
    pub(crate) suite: SuiteKind,
    pub(crate) file: &'a str,
    pub(crate) behaviour: Behaviour,
    pub(crate) pressure: Option<Pressure>,
    /// Passing runs with the skill.
    pub(crate) with: Count,
    /// Passing runs without it.
    pub(crate) base: Count,
    /// Runs that errored, both arms together.
    pub(crate) errored: u32,
    /// For a guard, whether all three runs with the skill passed (pass^3).
    pub(crate) guard_held: Option<bool>,
}

/// One suite's result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SuiteResult {
    pub(crate) suite: SuiteKind,
    pub(crate) cases: usize,
    pub(crate) with: Count,
    pub(crate) base: Count,
    /// The suite's own uplift interval, reported but not held to the bar.
    pub(crate) uplift: Option<Difference>,
    /// A capability suite whose every run with the skill passed.
    pub(crate) graduates: bool,
    /// A regression suite whose pass rate with the skill is confidently below 0.90.
    pub(crate) regressed: bool,
}

/// One skill's result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SkillResult<'a> {
    pub(crate) skill: &'a str,
    /// Passing runs with the skill, over all its cases.
    pub(crate) with: Count,
    /// Passing runs without it.
    pub(crate) base: Count,
    /// The uplift interval, or `None` when an arm has no trial.
    pub(crate) uplift: Option<Difference>,
    /// Whether the uplift's lower bound is 0 or more.
    pub(crate) uplift_met: bool,
    /// Its suites, regression first.
    pub(crate) suites: Vec<SuiteResult>,
    /// The files of its guard cases that did not hold pass^3.
    pub(crate) guards_failed: Vec<&'a str>,
    /// Whether it meets every bar.
    pub(crate) passed: bool,
}

/// An assertion that passed in 90% or more of its baseline runs: it does not tell the skill
/// from its absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NonDiscriminating<'a> {
    pub(crate) skill: &'a str,
    pub(crate) suite: SuiteKind,
    pub(crate) case: &'a str,
    pub(crate) grader: &'a str,
    /// Baseline runs it passed.
    pub(crate) passed: u32,
    /// Baseline runs graded.
    pub(crate) trials: u32,
}

/// A whole run's results (empty when nothing ran).
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub(crate) struct Tally<'a> {
    pub(crate) skills: Vec<SkillResult<'a>>,
    pub(crate) cases: Vec<CaseResult<'a>>,
    pub(crate) non_discriminating: Vec<NonDiscriminating<'a>>,
}

/// The results of `cases` given each case's runs by case name; a case with no entry has no
/// runs.
pub(crate) fn tally<'a>(cases: &'a [Case], runs: &BTreeMap<String, Arms>) -> Tally<'a> {
    let none = Arms::default();
    let mut results = Vec::with_capacity(cases.len());
    let mut non_discriminating = Vec::new();
    for case in cases {
        let arms = runs.get(&case.name()).unwrap_or(&none);
        results.push(case_result(case, arms));
        non_discriminating.extend(flags(case, &arms.base));
    }
    let mut by_skill: BTreeMap<&'a str, Vec<&CaseResult<'a>>> = BTreeMap::new();
    for result in &results {
        by_skill.entry(result.skill).or_default().push(result);
    }
    let skills = by_skill
        .into_iter()
        .map(|(skill, cases)| skill_result(skill, &cases))
        .collect();
    Tally {
        skills,
        cases: results,
        non_discriminating,
    }
}

fn case_result<'a>(case: &'a Case, arms: &Arms) -> CaseResult<'a> {
    let with = count(case, &arms.with);
    let errored = arms
        .with
        .iter()
        .chain(&arms.base)
        .filter(|trial| trial.errored())
        .count();
    CaseResult {
        skill: &case.skill,
        suite: case.suite,
        file: &case.file,
        behaviour: case.body.behaviour,
        pressure: case.body.pressure,
        with,
        base: count(case, &arms.base),
        errored: saturating_u32(errored),
        guard_held: (case.body.behaviour == Behaviour::Guard)
            .then_some(with.trials >= RUNS && with.passed == with.trials),
    }
}

fn skill_result<'a>(skill: &'a str, cases: &[&CaseResult<'a>]) -> SkillResult<'a> {
    let pooled = |of: fn(&CaseResult<'a>) -> Count, suite: Option<SuiteKind>| {
        cases
            .iter()
            .filter(|case| suite.is_none_or(|suite| case.suite == suite))
            .map(|case| of(case))
            .fold(Count::new(0, 0), Count::plus)
    };
    let suites: Vec<SuiteResult> = [SuiteKind::Regression, SuiteKind::Capability]
        .into_iter()
        .filter_map(|suite| {
            let n = cases.iter().filter(|case| case.suite == suite).count();
            (n > 0).then(|| {
                let (with, base) = (
                    pooled(|c| c.with, Some(suite)),
                    pooled(|c| c.base, Some(suite)),
                );
                SuiteResult {
                    suite,
                    cases: n,
                    with,
                    base,
                    uplift: newcombe(with, base),
                    graduates: suite == SuiteKind::Capability
                        && with.trials > 0
                        && with.passed == with.trials,
                    regressed: suite == SuiteKind::Regression
                        && wilson(with.passed, with.trials)
                            .is_some_and(|interval| interval.upper() < REGRESSION_MIN_UPPER),
                }
            })
        })
        .collect();
    let (with, base) = (pooled(|c| c.with, None), pooled(|c| c.base, None));
    let uplift = newcombe(with, base);
    let uplift_met = uplift.is_some_and(|u| u.lower() >= UPLIFT_MIN_LOWER);
    let guards_failed: Vec<&'a str> = cases
        .iter()
        .filter(|case| case.guard_held == Some(false))
        .map(|case| case.file)
        .collect();
    let passed =
        uplift_met && guards_failed.is_empty() && !suites.iter().any(|suite| suite.regressed);
    SkillResult {
        skill,
        with,
        base,
        uplift,
        uplift_met,
        suites,
        guards_failed,
        passed,
    }
}

/// The case's assertions that passed in 90% or more of its graded baseline runs.
fn flags<'a>(case: &'a Case, base: &[Trial]) -> Vec<NonDiscriminating<'a>> {
    case.body
        .graders
        .iter()
        .filter_map(|grader| {
            let name = grader.name();
            let results: Vec<bool> = base.iter().filter_map(|trial| trial.grader(name)).collect();
            let trials = saturating_u32(results.len());
            let passed = saturating_u32(results.iter().filter(|passed| **passed).count());
            // passed / trials >= 0.9, in integers.
            let flagged = trials > 0
                && u64::from(passed).saturating_mul(10) >= u64::from(trials).saturating_mul(9);
            flagged.then_some(NonDiscriminating {
                skill: &case.skill,
                suite: case.suite,
                case: &case.file,
                grader: name,
                passed,
                trials,
            })
        })
        .collect()
}

/// The passing runs and trials of `trials`, each judged against `case`'s graders.
fn count(case: &Case, trials: &[Trial]) -> Count {
    trials
        .iter()
        .filter_map(|trial| trial.passed(&case.body.graders))
        .fold(Count::new(0, 0), |count, passed| {
            count.plus(Count::new(u32::from(passed), 1))
        })
}

fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
