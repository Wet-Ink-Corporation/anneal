use std::collections::BTreeMap;

use serde_json::json;

use super::{Arm, Baseline, Batch, Evaluator, Ran, drive};
use crate::eval::routing::real::{ResultError, RouterError, Stop};
use crate::eval::uplift::Trial;
use crate::eval::uplift::case::parse;
use crate::eval::uplift::suites::{Case, SuiteKind};

fn case(skill: &str, slug: &str) -> Case {
    let text = json!({
        "behaviour": "outcome",
        "seed": "s",
        "prompt": "p",
        "graders": [{"type": "regex", "name": "a", "pattern": "x"}]
    })
    .to_string();
    Case {
        skill: skill.to_owned(),
        suite: SuiteKind::Regression,
        slug: slug.to_owned(),
        file: format!("{skill}/regression/{slug}.json"),
        body: parse(text.as_bytes()).expect("valid"),
    }
}

fn cases() -> Vec<Case> {
    vec![case("plan", "a"), case("plan", "b"), case("spec", "c")]
}

/// What a fake batch answers.
enum Answer {
    /// Every case passes with the skill and fails without it, at this cost.
    Costs(f64),
    /// The batch stops short for this reason, with no runs and no cost.
    Stops(Stop),
    /// The batch fails.
    Fails,
    /// Like `Costs`, but every case reports one run more than the batch asked for.
    Pads(f64),
    /// Like `Costs`, plus runs of `spec--regression--c` whether the batch asked for it or not.
    Strays(f64),
    /// The result was refused after its cost was read: a run named grader `invented`.
    Refuses(f64),
}

/// An evaluator that answers batch `i` with `answers[i]` (the last answer repeats) and
/// records each batch as `(arm, case names, runs, budget)`.
struct Fake {
    answers: Vec<Answer>,
    batches: Vec<(String, Vec<String>, u32, f64)>,
}

impl Fake {
    fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers,
            batches: Vec::new(),
        }
    }
}

impl Evaluator for Fake {
    fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError> {
        let arm = match batch.arm {
            Arm::With => "with".to_owned(),
            Arm::Without(skill) => format!("without {skill}"),
            Arm::Baseline => "baseline".to_owned(),
        };
        let names: Vec<String> = batch.cases.iter().map(|case| case.name()).collect();
        let answer = self
            .answers
            .get(self.batches.len())
            .or_else(|| self.answers.last())
            .expect("an answer");
        self.batches
            .push((arm, names.clone(), batch.runs, batch.budget_usd));
        let passed = matches!(batch.arm, Arm::With);
        let graded = |cost: f64, extra: u32| Ran {
            runs: names
                .iter()
                .map(|name| {
                    let run = Trial::Graded(vec![("a".to_owned(), passed)]);
                    let count = usize::try_from(batch.runs + extra).expect("small");
                    (name.clone(), vec![run; count])
                })
                .collect(),
            cost_usd: cost,
            cost_is_lower_bound: false,
            stop: None,
            refused: None,
        };
        match answer {
            Answer::Costs(cost) => Ok(graded(*cost, 0)),
            Answer::Pads(cost) => Ok(graded(*cost, 1)),
            Answer::Strays(cost) => {
                let mut ran = graded(*cost, 0);
                let run = Trial::Graded(vec![("a".to_owned(), passed)]);
                ran.runs
                    .insert("spec--regression--c".to_owned(), vec![run; 3]);
                Ok(ran)
            }
            Answer::Stops(stop) => Ok(Ran {
                runs: BTreeMap::new(),
                cost_usd: 0.0,
                cost_is_lower_bound: *stop == Stop::TimeLimit,
                stop: Some(stop.clone()),
                refused: None,
            }),
            Answer::Refuses(cost) => Ok(Ran {
                runs: BTreeMap::new(),
                cost_usd: *cost,
                cost_is_lower_bound: false,
                stop: None,
                refused: Some(ResultError::UnknownGrader {
                    case: "plan--regression--a".to_owned(),
                    grader: "invented".to_owned(),
                }),
            }),
            Answer::Fails => Err(RouterError::Failed {
                status: "exit status: 1".to_owned(),
                stderr: "boom".to_owned(),
            }),
        }
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn the_with_arm_runs_first_then_one_baseline_batch_per_skill_with_the_budget_left() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0)]);

    let drove = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0).expect("runs");

    assert_eq!(
        fake.batches,
        [
            (
                "with".to_owned(),
                names(&[
                    "plan--regression--a",
                    "plan--regression--b",
                    "spec--regression--c"
                ]),
                3,
                5.0
            ),
            (
                "without plan".to_owned(),
                names(&["plan--regression--a", "plan--regression--b"]),
                3,
                9.0
            ),
            (
                "without spec".to_owned(),
                names(&["spec--regression--c"]),
                3,
                8.0
            ),
        ]
    );
    assert_eq!((drove.spent_usd, drove.batches, drove.stop), (3.0, 3, None));
    let plan_a = drove.runs.get("plan--regression--a").expect("runs");
    let graders = &cases.first().expect("a case").body.graders;
    assert_eq!(
        (
            plan_a
                .with
                .iter()
                .filter_map(|trial| trial.passed(graders))
                .collect::<Vec<_>>(),
            plan_a
                .base
                .iter()
                .filter_map(|trial| trial.passed(graders))
                .collect::<Vec<_>>()
        ),
        (vec![true; 3], vec![false; 3])
    );
}

#[test]
fn a_baseline_plugin_runs_every_case_in_one_batch() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(0.5)]);

    let drove = drive(&mut fake, &cases, Baseline::Plugin, 10.0).expect("runs");

    let arms: Vec<(&str, usize)> = fake
        .batches
        .iter()
        .map(|(arm, names, _, _)| (arm.as_str(), names.len()))
        .collect();
    assert_eq!(arms, [("with", 3), ("baseline", 3)]);
    assert!((drove.spent_usd - 1.0).abs() < 1e-12, "{}", drove.spent_usd);
}

#[test]
fn no_batch_starts_with_less_than_a_cent_of_the_cap_left() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0)]);

    let drove = drive(&mut fake, &cases, Baseline::SkillRemoved, 1.005).expect("runs");

    assert_eq!(
        (fake.batches.len(), drove.batches, drove.stop),
        (1, 1, Some(Stop::CostCap))
    );
}

#[test]
fn a_batch_that_stops_short_ends_the_run_with_its_reason() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0), Answer::Stops(Stop::TimeLimit)]);

    let drove = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0).expect("runs");

    assert_eq!(
        (
            fake.batches.len(),
            drove.stop,
            drove.cost_is_lower_bound,
            drove
                .runs
                .get("plan--regression--a")
                .map(|arms| arms.base.len())
        ),
        (2, Some(Stop::TimeLimit), true, Some(0))
    );
}

#[test]
fn a_first_batch_failure_is_an_error_and_a_later_one_ends_the_run_as_failed() {
    let cases = cases();
    let mut first = Fake::new(vec![Answer::Fails]);
    let mut later = Fake::new(vec![Answer::Costs(1.0), Answer::Fails]);

    let failed_first = drive(&mut first, &cases, Baseline::SkillRemoved, 10.0);
    let failed_later = drive(&mut later, &cases, Baseline::SkillRemoved, 10.0).expect("runs");

    assert!(
        matches!(failed_first, Err(RouterError::Failed { .. })),
        "{failed_first:?}"
    );
    assert!(
        matches!(&failed_later.stop, Some(Stop::Failed { reason }) if reason.contains("boom")),
        "{:?}",
        failed_later.stop
    );
    assert_eq!(later.batches.len(), 2, "nothing after the failure");
}

#[test]
fn the_with_arm_may_spend_at_most_half_the_cap_so_the_baseline_keeps_the_rest() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0)]);

    drive(&mut fake, &cases, Baseline::Plugin, 10.0).expect("runs");

    let budgets: Vec<(&str, f64)> = fake
        .batches
        .iter()
        .map(|(arm, _, _, budget)| (arm.as_str(), *budget))
        .collect();
    assert_eq!(budgets, [("with", 5.0), ("baseline", 9.0)]);
}

#[test]
fn runs_of_a_case_the_batch_did_not_ask_for_are_dropped() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Strays(1.0)]);

    let drove = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0).expect("runs");

    assert_eq!(
        drove
            .runs
            .get("spec--regression--c")
            .map(|arms| (arms.with.len(), arms.base.len())),
        Some((3, 3)),
        "the without-plan batch's stray spec runs are not spec's baseline"
    );
}

#[test]
fn a_later_failure_leaves_the_cost_a_lower_bound() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0), Answer::Fails]);

    let drove = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0).expect("runs");

    assert_eq!(
        (drove.spent_usd, drove.batches, drove.cost_is_lower_bound),
        (1.0, 1, true),
        "claude may have spent before it failed"
    );
}

#[test]
fn a_first_batch_reporting_more_runs_than_asked_is_refused() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Pads(1.0)]);

    let refused = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0);

    assert!(
        matches!(
            &refused,
            Err(RouterError::Result(ResultError::TooManyRuns { case, runs: 4, asked: 3 }))
                if case == "plan--regression--a"
        ),
        "{refused:?}"
    );
}

#[test]
fn a_later_batch_reporting_more_runs_than_asked_ends_the_run_keeping_its_cost_not_its_runs() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0), Answer::Pads(2.0)]);

    let drove = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0).expect("runs");

    assert_eq!(
        (
            fake.batches.len(),
            drove.batches,
            drove.spent_usd,
            drove.cost_is_lower_bound,
            drove
                .runs
                .get("plan--regression--a")
                .map(|arms| (arms.with.len(), arms.base.len())),
        ),
        (2, 2, 3.0, false, Some((3, 0)))
    );
    assert!(
        matches!(&drove.stop, Some(Stop::Failed { reason }) if reason.contains("asked for 3")),
        "{:?}",
        drove.stop
    );
}

#[test]
fn no_cases_run_no_batch() {
    let mut fake = Fake::new(vec![Answer::Costs(1.0)]);

    let drove = drive(&mut fake, &[], Baseline::Plugin, 10.0).expect("runs");

    assert_eq!(
        (fake.batches.len(), drove.batches, drove.spent_usd),
        (0, 0, 0.0),
        "claude is never started on an empty eval directory"
    );
}

#[test]
fn a_first_batch_whose_result_is_refused_is_an_error() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Refuses(1.0)]);

    let refused = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0);

    assert!(
        matches!(
            &refused,
            Err(RouterError::Result(ResultError::UnknownGrader { grader, .. }))
                if grader == "invented"
        ),
        "{refused:?}"
    );
}

#[test]
fn a_later_batch_whose_result_is_refused_ends_the_run_keeping_its_cost() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0), Answer::Refuses(2.0)]);

    let drove = drive(&mut fake, &cases, Baseline::SkillRemoved, 10.0).expect("runs");

    assert_eq!(
        (
            fake.batches.len(),
            drove.batches,
            drove.spent_usd,
            drove.cost_is_lower_bound,
        ),
        (2, 2, 3.0, false),
        "the refused batch reported $2, so the spend stays exact"
    );
    assert!(
        matches!(&drove.stop, Some(Stop::Failed { reason }) if reason.contains("invented")),
        "{:?}",
        drove.stop
    );
}
