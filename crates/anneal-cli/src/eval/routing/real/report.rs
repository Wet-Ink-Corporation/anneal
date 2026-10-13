//! The envelope of `eval routing --router claude`.

use std::io::Write;

use serde::Serialize;

use super::schedule::{Count, Next, next};
use super::tally::{Probe, Rate, SkillResult, confusion, failed, per_skill};
use super::{RealOutcome, Run, Stop};
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::eval::problem_finding;
use crate::eval::real::wilson::Bar;
use crate::eval::routing::Confusion;
use crate::exit_codes::{self, Code};

/// The envelope's `data`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    router: &'static str,
    skills: usize,
    prompts: usize,
    runs: usize,
    errored: usize,
    counted_errors: u32,
    rounds: u32,
    #[serde(flatten)]
    spend: Spend,
    complete: bool,
    unsettled: usize,
    stop: Option<&'a Stop>,
    skipped: bool,
    problems: usize,
    per_skill: &'a [SkillResult<'a>],
    cases: Vec<CaseRow<'a>>,
    confusion: &'a Confusion<'a>,
    table: Vec<String>,
}

/// What the run spent, flattened into `data`.
#[derive(Debug, Serialize)]
struct Spend {
    /// As the router reported it.
    cost_usd: f64,
    /// A batch was killed before it reported its cost, so `cost_usd` is less than was spent.
    cost_is_lower_bound: bool,
    /// The cap.
    max_cost_usd: f64,
}

/// One prompt's line of the report.
#[derive(Debug, Serialize)]
struct CaseRow<'a> {
    file: &'a str,
    line: usize,
    skill: &'a str,
    runs: usize,
    fired: usize,
    errored: usize,
    next: Next,
}

impl<'a> CaseRow<'a> {
    fn of(probe: &'a Probe) -> Self {
        Self {
            file: &probe.file,
            line: probe.line,
            skill: &probe.skill,
            runs: probe.runs.len(),
            fired: probe
                .runs
                .iter()
                .filter(|run| run.fired(&probe.skill) == Some(true))
                .count(),
            errored: errored(probe),
            next: next(&probe.runs, &probe.skill),
        }
    }
}

fn errored(probe: &Probe) -> usize {
    probe
        .runs
        .iter()
        .filter(|run| matches!(run, Run::Errored))
        .count()
}

/// Writes the Tier 2 envelope for `outcome` to `out`.
///
/// # Errors
/// [`CliError::Output`] when `out` cannot be written.
pub(crate) fn report<O: Write>(outcome: &RealOutcome, out: &mut O) -> Result<Code, CliError> {
    let ran = outcome.drove.is_some();
    let results = if ran {
        per_skill(&outcome.skills, &outcome.probes)
    } else {
        Vec::new()
    };
    let unsettled = if ran {
        outcome
            .probes
            .iter()
            .filter(|probe| next(&probe.runs, &probe.skill) == Next::RunMore)
            .count()
    } else {
        0
    };
    let code = code(outcome, &results, unsettled);
    let table = confusion(&outcome.skills, &outcome.probes);
    let data = Data {
        router: "claude",
        skills: outcome.skills.len(),
        prompts: outcome.probes.len(),
        runs: outcome.probes.iter().map(|probe| probe.runs.len()).sum(),
        errored: outcome.probes.iter().map(errored).sum(),
        counted_errors: outcome
            .drove
            .as_ref()
            .map_or(0, |drove| drove.counted_errors),
        rounds: outcome.drove.as_ref().map_or(0, |drove| drove.rounds),
        spend: Spend {
            cost_usd: outcome.drove.as_ref().map_or(0.0, |drove| drove.spent_usd),
            cost_is_lower_bound: outcome
                .drove
                .as_ref()
                .is_some_and(|drove| drove.cost_is_lower_bound),
            max_cost_usd: outcome.max_cost_usd,
        },
        complete: ran && unsettled == 0,
        unsettled,
        stop: outcome.drove.as_ref().and_then(|drove| drove.stop.as_ref()),
        skipped: outcome.skipped,
        problems: outcome.problems.len(),
        per_skill: &results,
        cases: if ran {
            outcome.probes.iter().map(CaseRow::of).collect()
        } else {
            Vec::new()
        },
        confusion: &table,
        table: table.render(),
    };
    let findings = findings(outcome, &results, unsettled);
    Envelope::new(CommandName::Eval, Some("routing"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// [`exit_codes::EVAL_INVALID`] on any problem; [`exit_codes::OK`] when skipped;
/// [`exit_codes::EVAL_INCOMPLETE`] while a prompt is unsettled; then
/// [`exit_codes::EVAL_BELOW_BOUND`] when a skill misses a bar.
fn code(outcome: &RealOutcome, results: &[SkillResult<'_>], unsettled: usize) -> Code {
    if !outcome.problems.is_empty() {
        exit_codes::EVAL_INVALID
    } else if outcome.skipped {
        exit_codes::OK
    } else if unsettled > 0 {
        exit_codes::EVAL_INCOMPLETE
    } else if results.iter().any(|result| !result.passed) {
        exit_codes::EVAL_BELOW_BOUND
    } else {
        exit_codes::OK
    }
}

fn findings(outcome: &RealOutcome, results: &[SkillResult<'_>], unsettled: usize) -> Vec<Finding> {
    let mut findings: Vec<Finding> = outcome.problems.iter().map(problem_finding).collect();
    for result in results {
        findings.extend(
            outcome
                .probes
                .iter()
                .filter(|probe| probe.skill == result.skill && failed(probe))
                .map(prompt_finding),
        );
        if !result.should_not.met() {
            findings.push(should_not_finding(result.skill, result.should_not));
        }
    }
    if unsettled > 0 {
        let why = match outcome.drove.as_ref().and_then(|drove| drove.stop.as_ref()) {
            Some(Stop::CostCap) => format!("the ${:.2} cost cap was reached", outcome.max_cost_usd),
            Some(Stop::TimeLimit) => "the time limit was reached".to_owned(),
            Some(Stop::Interrupted) => "claude plugin eval reported its run interrupted".to_owned(),
            Some(Stop::Failed { reason }) => format!("a batch failed: {reason}"),
            None => "claude plugin eval returned fewer runs than asked".to_owned(),
        };
        findings.push(Finding::new(
            "eval/routing/incomplete",
            format!("{unsettled} prompts are unsettled: {why}"),
        ));
    }
    if let Some(reason) = &outcome.cleanup {
        findings.push(Finding::new(
            "eval/routing/cleanup",
            format!("the temporary eval workspace was not removed: {reason}"),
        ));
    }
    findings
}

/// The finding for a prompt that failed its should-fire bar.
fn prompt_finding(probe: &Probe) -> Finding {
    let rate = Rate::new(Bar::ShouldFire, Count::of(&probe.runs, &probe.skill));
    let place = format!("{}:{}", probe.file, probe.line);
    Finding::new(
        format!("eval/routing/should-fire:{}:{place}", probe.skill),
        format!(
            "`{}` fired in {} of {} finished runs of its prompt at {place}: {}",
            probe.skill,
            rate.count.fired,
            rate.count.trials,
            bound(rate)
        ),
    )
}

/// The finding for a skill that missed its pooled should-not bar.
fn should_not_finding(skill: &str, rate: Rate) -> Finding {
    Finding::new(
        format!("eval/routing/should-not:{skill}"),
        format!(
            "`{skill}` fired in {} of {} finished runs of other skills' prompts: {}",
            rate.count.fired,
            rate.count.trials,
            bound(rate)
        ),
    )
}

/// The bound of `rate`'s interval its bar is judged on, against the threshold.
fn bound(rate: Rate) -> String {
    rate.interval.map_or_else(
        || "no finished run, so no interval".to_owned(),
        |interval| {
            let (side, value) = match rate.bar {
                Bar::ShouldFire => ("lower", interval.lower()),
                Bar::ShouldNot => ("upper", interval.upper()),
            };
            format!(
                "95% Wilson {side} bound {value:.3} against {:.2}",
                rate.threshold
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::report;
    use crate::eval::routing::real::drive::Drove;
    use crate::eval::routing::real::tally::Probe;
    use crate::eval::routing::real::{RealOutcome, Run, Stop};

    fn outcome(runs: [Vec<Run>; 2], stop: Option<Stop>) -> RealOutcome {
        let [plan, spec] = runs;
        let probe = |skill: &str, runs| Probe {
            skill: skill.to_owned(),
            file: format!("{skill}.yaml"),
            line: 3,
            text: format!("a {skill} prompt"),
            runs,
        };
        RealOutcome {
            skills: vec!["plan".to_owned(), "spec".to_owned()],
            probes: vec![probe("plan", plan), probe("spec", spec)],
            problems: Vec::new(),
            drove: Some(Drove {
                spent_usd: 1.25,
                rounds: 1,
                stop,
                cost_is_lower_bound: false,
                counted_errors: 0,
            }),
            skipped: false,
            cleanup: None,
            max_cost_usd: 5.0,
        }
    }

    fn fired(skill: &str, n: usize) -> Vec<Run> {
        vec![
            Run::Done {
                fired: vec![skill.to_owned()],
                answered: true
            };
            n
        ]
    }

    fn emitted(outcome: &RealOutcome) -> Result<(u8, Value), Box<dyn std::error::Error>> {
        let mut out = Vec::new();
        let code = report(outcome, &mut out)?;
        Ok((code.value(), serde_json::from_slice(&out)?))
    }

    #[test]
    fn an_unsettled_prompt_makes_the_run_incomplete_and_names_the_stop()
    -> Result<(), Box<dyn std::error::Error>> {
        let (code, envelope) = emitted(&outcome(
            [fired("plan", 3), fired("spec", 12)],
            Some(Stop::CostCap),
        ))?;

        assert_eq!(code, 3, "{envelope}");
        assert_eq!(
            envelope["findings"]
                .as_array()
                .and_then(|f| f.iter().find(|f| f["key"] == "eval/routing/incomplete"))
                .map(|f| f["message"].clone()),
            Some("1 prompts are unsettled: the $5.00 cost cap was reached".into())
        );
        Ok(())
    }

    #[test]
    fn a_missed_bar_is_below_bound_with_one_finding_per_bar()
    -> Result<(), Box<dyn std::error::Error>> {
        // Both prompts fire `plan` 12 times, so `plan` misses its should-not bar and `spec` its
        // should-fire bar. And `spec`'s should-not rests on 12 runs alone (0 of 12, upper bound
        // 0.242), which cannot reach 0.20.
        let (code, envelope) = emitted(&outcome([fired("plan", 12), fired("plan", 12)], None))?;

        let keys: Vec<&str> = envelope["findings"]
            .as_array()
            .map(|f| f.iter().filter_map(|f| f["key"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(
            (code, keys),
            (
                1,
                vec![
                    "eval/routing/should-not:plan",
                    "eval/routing/should-fire:spec:spec.yaml:3",
                    "eval/routing/should-not:spec"
                ]
            )
        );
        Ok(())
    }

    #[test]
    fn a_failing_prompt_finding_names_its_place_and_interval()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_, envelope) = emitted(&outcome([fired("plan", 12), fired("plan", 12)], None))?;

        assert_eq!(
            envelope["findings"]
                .as_array()
                .and_then(|f| {
                    f.iter()
                        .find(|f| f["key"] == "eval/routing/should-fire:spec:spec.yaml:3")
                })
                .map(|f| &f["message"]),
            Some(&serde_json::json!(
                "`spec` fired in 0 of 12 finished runs of its prompt at spec.yaml:3: \
                 95% Wilson lower bound 0.000 against 0.70"
            ))
        );
        Ok(())
    }

    #[test]
    fn a_killed_batch_makes_the_cost_a_lower_bound_and_counted_errors_are_reported()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut killed = outcome([fired("plan", 3), fired("spec", 3)], Some(Stop::TimeLimit));
        if let Some(drove) = killed.drove.as_mut() {
            drove.cost_is_lower_bound = true;
            drove.counted_errors = 2;
        }

        let (_, envelope) = emitted(&killed)?;

        assert_eq!(
            (
                &envelope["data"]["cost_is_lower_bound"],
                &envelope["data"]["counted_errors"]
            ),
            (&serde_json::json!(true), &serde_json::json!(2))
        );
        Ok(())
    }

    #[test]
    fn every_bar_met_exits_0_and_reports_each_prompt() -> Result<(), Box<dyn std::error::Error>> {
        let mut passing = outcome([fired("plan", 12), fired("spec", 12)], None);
        passing.probes.extend(passing.probes.clone());

        let (code, envelope) = emitted(&passing)?;

        assert_eq!(code, 0, "{envelope}");
        assert_eq!(
            envelope["data"]["cases"][0],
            serde_json::json!({"file": "plan.yaml", "line": 3, "skill": "plan", "runs": 12, "fired": 12, "errored": 0, "next": "met"})
        );
        Ok(())
    }
}
