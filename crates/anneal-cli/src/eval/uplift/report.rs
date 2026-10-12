//! The envelope of `eval uplift`.

use std::collections::BTreeSet;
use std::io::Write;

use serde::Serialize;

use super::drive::Drove;
use super::stats::Difference;
use super::suites::{Coverage, SuiteKind};
use super::tally::{CaseResult, NonDiscriminating, RUNS, SkillResult, Tally, tally};
use super::{BaselineKind, Trial, UpliftOutcome};
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::eval::routing::real::Stop;
use crate::eval::routing::real::wilson::wilson;
use crate::exit_codes::{self, Code};

/// The envelope's `data`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    mode: &'static str,
    skills: Vec<&'a str>,
    cases: usize,
    runs_per_arm: u32,
    runs: usize,
    errored: usize,
    #[serde(flatten)]
    spend: Spend,
    baseline: Option<&'a str>,
    baseline_plugin: Option<&'a str>,
    complete: bool,
    stop: Option<&'a Stop>,
    skipped: bool,
    problems: usize,
    coverage: &'a [Coverage],
    per_skill: &'a [SkillResult<'a>],
    results: &'a [CaseResult<'a>],
    non_discriminating: &'a [NonDiscriminating<'a>],
    graduates: Vec<&'a str>,
}

/// What the run spent, flattened into `data`.
#[derive(Debug, Serialize)]
struct Spend {
    /// As `claude plugin eval` reported it.
    cost_usd: f64,
    /// A batch was killed before it reported its cost, so `cost_usd` is less than was spent.
    cost_is_lower_bound: bool,
    /// The cap, when a run was asked for.
    max_cost_usd: Option<f64>,
}

/// Writes the Tier 3 envelope for `outcome` to `out`.
///
/// # Errors
/// [`CliError::Output`] when `out` cannot be written.
pub(crate) fn report<O: Write>(outcome: &UpliftOutcome, out: &mut O) -> Result<Code, CliError> {
    let results = outcome.drove.as_ref().map_or_else(Tally::default, |drove| {
        tally(&outcome.suites.cases, &drove.runs)
    });
    let short = outcome.drove.as_ref().map_or(0, short_cases);
    let code = code(outcome, &results, short);
    let findings = findings(outcome, &results, short);
    Envelope::new(
        CommandName::Eval,
        Some("uplift"),
        code,
        data(outcome, &results, short),
    )
    .with_findings(&findings)
    .emit(out)
    .map_err(CliError::Output)
}

fn data<'a>(outcome: &'a UpliftOutcome, results: &'a Tally<'a>, short: usize) -> Data<'a> {
    let trials: Vec<&Trial> = outcome
        .drove
        .iter()
        .flat_map(|drove| drove.runs.values())
        .flat_map(|arms| arms.with.iter().chain(&arms.base))
        .collect();
    let skills: BTreeSet<&str> = outcome
        .suites
        .cases
        .iter()
        .map(|case| case.skill.as_str())
        .collect();
    let (baseline, baseline_plugin) = match &outcome.baseline {
        None => (None, None),
        Some(BaselineKind::SkillRemoved) => (Some("skill-removed"), None),
        Some(BaselineKind::Plugin(dir)) => (Some("plugin"), Some(dir.as_str())),
    };
    let drove = outcome.drove.as_ref();
    Data {
        mode: if outcome.check { "check" } else { "run" },
        skills: skills.into_iter().collect(),
        cases: outcome.suites.cases.len(),
        runs_per_arm: RUNS,
        runs: trials.len(),
        errored: trials
            .iter()
            .filter(|trial| trial.passed().is_none())
            .count(),
        spend: Spend {
            cost_usd: drove.map_or(0.0, |drove| drove.spent_usd),
            cost_is_lower_bound: drove.is_some_and(|drove| drove.cost_is_lower_bound),
            max_cost_usd: outcome.max_cost_usd,
        },
        baseline,
        baseline_plugin,
        complete: drove.is_some_and(|drove| drove.stop.is_none() && short == 0),
        stop: drove.and_then(|drove| drove.stop.as_ref()),
        skipped: outcome.skipped,
        problems: outcome.suites.problems.len(),
        coverage: &outcome.suites.coverage,
        per_skill: &results.skills,
        results: &results.cases,
        non_discriminating: &results.non_discriminating,
        graduates: results
            .skills
            .iter()
            .filter(|skill| skill.suites.iter().any(|suite| suite.graduates))
            .map(|skill| skill.skill)
            .collect(),
    }
}

/// How many cases have fewer runs than were asked for in an arm (errored runs count as
/// runs).
fn short_cases(drove: &Drove) -> usize {
    let short = |trials: &[Trial]| u32::try_from(trials.len()).is_ok_and(|runs| runs < RUNS);
    drove
        .runs
        .values()
        .filter(|arms| short(&arms.with) || short(&arms.base))
        .count()
}

/// [`exit_codes::EVAL_INVALID`] on any problem; [`exit_codes::OK`] for `--check` or when
/// skipped; [`exit_codes::EVAL_INCOMPLETE`] when the run stopped short or `short` cases lack
/// runs in an arm; then [`exit_codes::EVAL_BELOW_BOUND`] when a skill misses a bar.
fn code(outcome: &UpliftOutcome, results: &Tally<'_>, short: usize) -> Code {
    if !outcome.suites.problems.is_empty() {
        return exit_codes::EVAL_INVALID;
    }
    let Some(drove) = &outcome.drove else {
        return exit_codes::OK;
    };
    if drove.stop.is_some() || short > 0 {
        exit_codes::EVAL_INCOMPLETE
    } else if results.skills.iter().any(|skill| !skill.passed) {
        exit_codes::EVAL_BELOW_BOUND
    } else {
        exit_codes::OK
    }
}

/// Every finding: suite problems, each skill's missed bars, non-discriminating assertions,
/// then why the run is incomplete and a workspace left behind.
fn findings(outcome: &UpliftOutcome, results: &Tally<'_>, short: usize) -> Vec<Finding> {
    let mut findings: Vec<Finding> = outcome
        .suites
        .problems
        .iter()
        .map(|problem| {
            Finding::new(
                format!("eval/uplift/suite:{}", problem.file),
                format!("{}: {}", problem.file, problem.reason),
            )
        })
        .collect();
    for skill in &results.skills {
        findings.extend(skill_findings(skill, &results.cases));
    }
    findings.extend(results.non_discriminating.iter().map(|flag| {
        Finding::new(
            format!(
                "eval/uplift/non-discriminating:{}:{}",
                flag.case, flag.grader
            ),
            format!(
                "assertion `{}` of {} passed {} of {} baseline runs: it does not tell the \
                 skill from its absence",
                flag.grader, flag.case, flag.passed, flag.trials
            ),
        )
    }));
    findings.extend(incomplete_finding(outcome, short));
    if let Some(reason) = &outcome.cleanup {
        findings.push(Finding::new(
            "eval/uplift/cleanup",
            format!("the temporary eval workspace was not removed: {reason}"),
        ));
    }
    findings
}

/// The findings of one skill's missed bars: its uplift, each guard case that did not hold
/// pass^3, and a regressed regression suite.
fn skill_findings(skill: &SkillResult<'_>, cases: &[CaseResult<'_>]) -> Vec<Finding> {
    let mut findings = Vec::new();
    if !skill.uplift_met {
        findings.push(Finding::new(
            format!("eval/uplift/uplift:{}", skill.skill),
            format!(
                "`{}` passed {} of {} runs with the skill and {} of {} without: {}",
                skill.skill,
                skill.with.passed,
                skill.with.trials,
                skill.base.passed,
                skill.base.trials,
                uplift_bound(skill.uplift)
            ),
        ));
    }
    findings.extend(
        cases
            .iter()
            .filter(|case| case.skill == skill.skill && case.guard_held == Some(false))
            .map(|case| {
                Finding::new(
                    format!("eval/uplift/guard:{}", case.file),
                    format!(
                        "guard case {} passed {} of {} runs with the skill; a guard must pass \
                         all {RUNS} (pass^{RUNS})",
                        case.file, case.with.passed, case.with.trials
                    ),
                )
            }),
    );
    findings.extend(
        skill
            .suites
            .iter()
            .filter(|suite| suite.regressed)
            .map(|suite| {
                // A suite regresses only on an interval, so there is one to name.
                let bound = wilson(suite.with.passed, suite.with.trials).map_or_else(
                    String::new,
                    |interval| {
                        format!(
                            ": 95% Wilson upper bound {:.3} below 0.90",
                            interval.upper()
                        )
                    },
                );
                Finding::new(
                    format!("eval/uplift/regressed:{}", skill.skill),
                    format!(
                        "the {} suite of `{}` passed {} of {} runs with the skill{bound}",
                        SuiteKind::Regression.dir(),
                        skill.skill,
                        suite.with.passed,
                        suite.with.trials
                    ),
                )
            }),
    );
    findings
}

/// Why the run is incomplete, if it is: it stopped early, or `short` cases lack runs.
fn incomplete_finding(outcome: &UpliftOutcome, short: usize) -> Option<Finding> {
    let why = match outcome.drove.as_ref().and_then(|drove| drove.stop.as_ref()) {
        Some(Stop::CostCap) => outcome.max_cost_usd.map_or_else(
            || "the cost cap was reached".to_owned(),
            |cap| format!("the ${cap:.2} cost cap was reached"),
        ),
        Some(Stop::TimeLimit) => "the time limit was reached".to_owned(),
        Some(Stop::Interrupted) => "claude plugin eval reported its run interrupted".to_owned(),
        Some(Stop::Failed { reason }) => format!("a batch failed: {reason}"),
        None if short > 0 => {
            return Some(Finding::new(
                "eval/uplift/incomplete",
                format!(
                    "{short} cases have fewer than {RUNS} runs in an arm: claude plugin eval \
                     returned fewer runs than asked"
                ),
            ));
        }
        None => return None,
    };
    Some(Finding::new(
        "eval/uplift/incomplete",
        format!("the run stopped before every batch ran: {why}"),
    ))
}

/// The uplift's interval against the bar, or why there is none.
fn uplift_bound(uplift: Option<Difference>) -> String {
    uplift.map_or_else(
        || "no interval, since an arm has no finished run".to_owned(),
        |uplift| {
            format!(
                "uplift {:.3}, 95% Newcombe interval {:.3} to {:.3}, whose lower bound is below 0",
                uplift.estimate(),
                uplift.lower(),
                uplift.upper()
            )
        },
    )
}

#[cfg(test)]
mod tests;
