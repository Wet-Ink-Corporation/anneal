use std::collections::BTreeMap;

use serde_json::json;

use super::{Arms, NonDiscriminating, tally};
use crate::eval::uplift::Trial;
use crate::eval::uplift::case::{Behaviour, parse};
use crate::eval::uplift::stats::Difference;
use crate::eval::uplift::suites::{Case, SuiteKind};

/// A case of `skill` in `suite` with graders `a` and `b`.
fn case(skill: &str, suite: SuiteKind, slug: &str, behaviour: Behaviour) -> Case {
    let behaviour = match behaviour {
        Behaviour::Guard => "guard",
        Behaviour::Outcome => "outcome",
    };
    let text = json!({
        "behaviour": behaviour,
        "seed": "s",
        "prompt": "p",
        "graders": [
            {"type": "regex", "name": "a", "pattern": "x"},
            {"type": "regex", "name": "b", "pattern": "y"}
        ]
    })
    .to_string();
    Case {
        skill: skill.to_owned(),
        suite,
        slug: slug.to_owned(),
        file: format!("{skill}/{}/{slug}.json", suite.dir()),
        body: parse(text.as_bytes()).expect("valid"),
    }
}

/// A graded run where grader `a` passed or not, and `b` passed or not.
fn run([a, b]: [bool; 2]) -> Trial {
    Trial::Graded(vec![("a".to_owned(), a), ("b".to_owned(), b)])
}

fn pass() -> Trial {
    run([true, true])
}

fn fail() -> Trial {
    run([false, false])
}

fn arms(with: Vec<Trial>, base: Vec<Trial>) -> Arms {
    Arms { with, base }
}

#[test]
fn a_run_passes_only_when_every_grader_passed_and_an_errored_run_is_no_trial() {
    let graders = &case("plan", SuiteKind::Regression, "o", Behaviour::Outcome)
        .body
        .graders;

    assert_eq!(
        [
            pass().passed(graders),
            run([true, false]).passed(graders),
            Trial::Graded(Vec::new()).passed(graders),
            Trial::Errored.passed(graders)
        ],
        [Some(true), Some(false), Some(false), None]
    );
}

#[test]
fn a_run_that_leaves_out_one_of_the_cases_graders_does_not_pass() {
    let outcome = case("plan", SuiteKind::Regression, "o", Behaviour::Outcome);
    let only_a = || Trial::Graded(vec![("a".to_owned(), true)]);
    let runs = BTreeMap::from([(
        outcome.name(),
        arms(
            vec![only_a(), only_a(), only_a()],
            vec![fail(), fail(), fail()],
        ),
    )]);

    let tallied = tally(std::slice::from_ref(&outcome), &runs);

    assert_eq!(
        tallied
            .cases
            .first()
            .map(|result| (result.with.passed, result.with.trials)),
        Some((0, 3)),
        "grader b never reported, so no run passed"
    );
}

#[test]
fn a_guard_holds_only_when_all_three_runs_pass() {
    let guard = case("plan", SuiteKind::Regression, "g", Behaviour::Guard);
    let outcomes = [
        (vec![pass(), pass(), pass()], Some(true)),
        (vec![pass(), pass(), fail()], Some(false)),
        (vec![pass(), pass(), Trial::Errored], Some(false)),
    ];

    for (with, held) in outcomes {
        let runs = BTreeMap::from([(guard.name(), arms(with, vec![fail(), fail(), fail()]))]);

        let tallied = tally(std::slice::from_ref(&guard), &runs);

        assert_eq!(
            tallied.cases.first().and_then(|c| c.guard_held),
            held,
            "{:?}",
            tallied.cases.first()
        );
    }
}

#[test]
fn an_outcome_case_has_no_guard_verdict() {
    let outcome = case("plan", SuiteKind::Regression, "o", Behaviour::Outcome);
    let runs = BTreeMap::from([(outcome.name(), arms(vec![pass()], vec![fail()]))]);

    let tallied = tally(std::slice::from_ref(&outcome), &runs);

    assert_eq!(tallied.cases.first().map(|c| c.guard_held), Some(None));
}

#[test]
fn an_assertion_passing_in_90_percent_or_more_of_baseline_runs_is_non_discriminating() {
    let plan = case("plan", SuiteKind::Capability, "c", Behaviour::Outcome);
    let base = vec![
        run([true, true]),
        run([true, false]),
        run([true, true]),
        Trial::Errored,
    ];
    let runs = BTreeMap::from([(plan.name(), arms(vec![pass(), pass(), pass()], base))]);

    let tallied = tally(std::slice::from_ref(&plan), &runs);

    assert_eq!(
        tallied.non_discriminating,
        [NonDiscriminating {
            skill: "plan",
            suite: SuiteKind::Capability,
            case: "plan/capability/c.json",
            grader: "a",
            passed: 3,
            trials: 3,
        }],
        "`b` passed 2 of 3 baseline runs; the errored run is no trial"
    );
}

#[test]
fn the_uplift_is_pooled_over_a_skills_cases_and_met_at_a_lower_bound_of_zero() {
    let cases = [
        case("build", SuiteKind::Regression, "r", Behaviour::Outcome),
        case("plan", SuiteKind::Regression, "r", Behaviour::Outcome),
        case("plan", SuiteKind::Capability, "c", Behaviour::Outcome),
    ];
    let three = |trial: fn() -> Trial| vec![trial(), trial(), trial()];
    let runs = BTreeMap::from([
        (cases[0].name(), arms(three(pass), three(pass))),
        (cases[1].name(), arms(three(pass), three(fail))),
        (cases[2].name(), arms(three(pass), three(fail))),
    ]);

    let tallied = tally(&cases, &runs);

    let skills: Vec<(&str, u32, u32, u32, u32, bool)> = tallied
        .skills
        .iter()
        .map(|s| {
            (
                s.skill,
                s.with.passed,
                s.with.trials,
                s.base.passed,
                s.base.trials,
                s.uplift_met,
            )
        })
        .collect();
    assert_eq!(
        skills,
        [("build", 3, 3, 3, 3, false), ("plan", 6, 6, 0, 6, true),],
        "build helps no more than its baseline; plan's 6 of 6 against 0 of 6 bounds above zero"
    );
    let plan_lower = tallied
        .skills
        .get(1)
        .and_then(|s| s.uplift)
        .map(Difference::lower);
    // 6 of 6 against 0 of 6: Newcombe's lower bound, computed independently in Python.
    assert!(
        plan_lower.is_some_and(|lower| (lower - 0.447_983_956_2).abs() < 1e-9),
        "{plan_lower:?}"
    );
}

#[test]
fn a_capability_suite_at_100_percent_graduates_and_a_regression_suite_below_90_regressed() {
    let cases = [
        case("plan", SuiteKind::Capability, "c", Behaviour::Outcome),
        case("plan", SuiteKind::Regression, "r", Behaviour::Outcome),
        case("spec", SuiteKind::Regression, "r", Behaviour::Outcome),
    ];
    let runs = BTreeMap::from([
        (
            cases[0].name(),
            arms(vec![pass(), pass(), pass()], vec![fail()]),
        ),
        (
            cases[1].name(),
            arms(vec![fail(), fail(), fail()], vec![fail()]),
        ),
        (
            cases[2].name(),
            arms(vec![pass(), pass(), fail()], vec![fail()]),
        ),
    ]);

    let tallied = tally(&cases, &runs);

    let suites: Vec<(&str, SuiteKind, bool, bool)> = tallied
        .skills
        .iter()
        .flat_map(|s| {
            s.suites
                .iter()
                .map(move |suite| (s.skill, suite.suite, suite.graduates, suite.regressed))
        })
        .collect();
    assert_eq!(
        suites,
        [
            ("plan", SuiteKind::Regression, false, true),
            ("plan", SuiteKind::Capability, true, false),
            ("spec", SuiteKind::Regression, false, false),
        ],
        "0 of 3 has an upper bound of 0.56, below 0.90; 2 of 3 has 0.94"
    );
}

#[test]
fn a_skill_passes_only_with_its_uplift_met_every_guard_held_and_no_regression() {
    let cases = [
        case("plan", SuiteKind::Regression, "g", Behaviour::Guard),
        case("plan", SuiteKind::Capability, "c", Behaviour::Outcome),
    ];
    let six_of_nine = vec![pass(), pass(), fail()];
    let runs = BTreeMap::from([
        (
            cases[0].name(),
            arms(six_of_nine, vec![fail(), fail(), fail()]),
        ),
        (
            cases[1].name(),
            arms(vec![pass(), pass(), pass()], vec![fail(), fail(), fail()]),
        ),
    ]);

    let tallied = tally(&cases, &runs);

    let plan = tallied.skills.first().expect("plan");
    assert_eq!(
        (plan.uplift_met, plan.guards_failed.as_slice(), plan.passed),
        (true, ["plan/regression/g.json"].as_slice(), false),
        "uplift met, but the guard did not hold all three runs"
    );
}

#[test]
fn a_case_with_no_runs_leaves_its_skill_with_no_interval_and_failing() {
    let plan = case("plan", SuiteKind::Regression, "r", Behaviour::Outcome);

    let tallied = tally(std::slice::from_ref(&plan), &BTreeMap::new());

    let skill = tallied.skills.first().expect("plan");
    assert_eq!((skill.uplift, skill.passed), (None, false));
}
