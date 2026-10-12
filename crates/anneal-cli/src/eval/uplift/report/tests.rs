use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::report;
use crate::eval::uplift::case::parse;
use crate::eval::uplift::drive::Drove;
use crate::eval::uplift::suites::{Case, SuiteKind, Suites};
use crate::eval::uplift::tally::Arms;
use crate::eval::uplift::{BaselineKind, Trial, UpliftOutcome};

/// A `plan` case in `suite` named `slug`, of `behaviour`, with one grader `a`.
fn case(suite: SuiteKind, slug: &str, behaviour: &str) -> Case {
    let text = json!({
        "behaviour": behaviour,
        "seed": "s",
        "prompt": "p",
        "graders": [{"type": "regex", "name": "a", "pattern": "x"}]
    })
    .to_string();
    Case {
        skill: "plan".to_owned(),
        suite,
        slug: slug.to_owned(),
        file: format!("plan/{}/{slug}.json", suite.dir()),
        body: parse(text.as_bytes()).expect("valid"),
    }
}

fn trials(passes: &[bool]) -> Vec<Trial> {
    passes
        .iter()
        .map(|passed| Trial::Graded(vec![("a".to_owned(), *passed)]))
        .collect()
}

/// A finished run of `cases` with these runs by case name.
fn outcome(cases: Vec<Case>, runs: Vec<(&str, Arms)>) -> UpliftOutcome {
    UpliftOutcome {
        check: false,
        suites: Suites {
            cases,
            problems: Vec::new(),
            coverage: Vec::new(),
        },
        drove: Some(Drove {
            runs: runs
                .into_iter()
                .map(|(name, arms)| (name.to_owned(), arms))
                .collect::<BTreeMap<_, _>>(),
            spent_usd: 1.0,
            batches: 2,
            stop: None,
            cost_is_lower_bound: false,
        }),
        skipped: false,
        baseline: Some(BaselineKind::SkillRemoved),
        cleanup: None,
        max_cost_usd: Some(25.0),
    }
}

/// The exit code and the finding keys of `outcome`'s envelope.
fn judged(outcome: &UpliftOutcome) -> (u8, Vec<String>, Value) {
    let mut out = Vec::new();
    let code = report(outcome, &mut out).expect("written");
    let mut envelope: Value = serde_json::from_slice(&out).expect("json");
    let keys = envelope["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["key"].as_str().expect("key").to_owned())
        .collect();
    (code.value(), keys, envelope["data"]["complete"].take())
}

#[test]
fn a_case_short_of_its_runs_in_an_arm_is_incomplete() {
    let outcome = outcome(
        vec![case(SuiteKind::Regression, "a", "outcome")],
        vec![(
            "plan--regression--a",
            Arms {
                with: trials(&[true, true, true]),
                base: trials(&[false]),
            },
        )],
    );

    assert_eq!(
        judged(&outcome),
        (3, vec!["eval/uplift/incomplete".to_owned()], json!(false))
    );
}

#[test]
fn a_guard_that_fails_one_of_three_runs_is_below_bound_with_its_finding() {
    let outcome = outcome(
        vec![
            case(SuiteKind::Regression, "a", "outcome"),
            case(SuiteKind::Capability, "g", "guard"),
        ],
        vec![
            (
                "plan--regression--a",
                Arms {
                    with: trials(&[true, true, true]),
                    base: trials(&[false, false, false]),
                },
            ),
            (
                "plan--capability--g",
                Arms {
                    with: trials(&[true, false, true]),
                    base: trials(&[false, false, false]),
                },
            ),
        ],
    );

    assert_eq!(
        judged(&outcome),
        (
            1,
            vec!["eval/uplift/guard:plan/capability/g.json".to_owned()],
            json!(true)
        )
    );
}

#[test]
fn a_regression_suite_far_below_90_percent_regressed() {
    let outcome = outcome(
        vec![case(SuiteKind::Regression, "a", "outcome")],
        vec![(
            "plan--regression--a",
            Arms {
                with: trials(&[true, false, false]),
                base: trials(&[false, false, false]),
            },
        )],
    );

    assert_eq!(
        judged(&outcome),
        (
            1,
            vec![
                "eval/uplift/uplift:plan".to_owned(),
                "eval/uplift/regressed:plan".to_owned()
            ],
            json!(true)
        )
    );
}
