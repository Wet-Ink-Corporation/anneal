use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::report;
use crate::eval::uplift::case::parse;
use crate::eval::uplift::drive::Drove;
use crate::eval::uplift::fixture::Fixture;
use crate::eval::uplift::suites::{Case, SuiteKind, Suites};
use crate::eval::uplift::tally::Arms;
use crate::eval::uplift::{BaselineKind, PlacedAnneal, Trial, UpliftOutcome};

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
        fixture: None,
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
        anneal: None,
        read_only_sandbox: None,
    }
}

#[test]
fn a_run_made_where_bash_cannot_write_says_so_and_how_to_move_it() {
    let mut outcome = outcome(Vec::new(), Vec::new());
    outcome.read_only_sandbox = Some("/tmp".to_owned());

    let mut out = Vec::new();
    report(&outcome, &mut out).expect("written");
    let envelope: Value = serde_json::from_slice(&out).expect("json");

    assert_eq!(
        envelope["findings"],
        json!([{
            "key": "eval/uplift/read-only-sandbox",
            "message": "the run's workspace was made in /tmp, under which claude plugin eval's \
                        Linux sandbox denies Bash every write (the anneal ledger's included), so \
                        the cases that name Bash could not write; run again with --work-dir (or \
                        TMPDIR) outside /tmp, /var/tmp, /dev/shm and /run/shm"
        }])
    );
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

#[test]
fn the_gated_tools_cases_grant_are_listed_per_case_and_named_in_a_finding() {
    let text = json!({
        "behaviour": "outcome",
        "seed": "s",
        "prompt": "p",
        "allowed_tools": ["Skill", "Bash", "Read", "Write"],
        "graders": [{"type": "regex", "name": "a", "pattern": "x"}]
    })
    .to_string();
    let gated = Case {
        body: parse(text.as_bytes()).expect("valid"),
        ..case(SuiteKind::Regression, "b", "outcome")
    };
    let mut outcome = outcome(
        vec![case(SuiteKind::Regression, "a", "outcome"), gated],
        Vec::new(),
    );
    outcome.check = true;
    outcome.drove = None;

    let mut out = Vec::new();
    report(&outcome, &mut out).expect("written");
    let envelope: Value = serde_json::from_slice(&out).expect("json");

    assert_eq!(
        (&envelope["data"]["granted_tools"], &envelope["findings"]),
        (
            &json!([{"case": "plan/regression/b.json", "tools": ["Write", "Bash"]}]),
            &json!([{
                "key": "eval/uplift/granted-tools",
                "message": "a run grants Write, Bash to 1 case from its case file alone \
                            (plan/regression/b.json); review those cases before running them"
            }])
        )
    );
}

/// A `plan` regression case named `slug` seeded from the fixture `fixture`.
fn seeded(slug: &str, fixture: &str) -> Case {
    Case {
        fixture: Some(Fixture::sample(fixture)),
        ..case(SuiteKind::Regression, slug, "outcome")
    }
}

#[test]
fn the_cases_seeded_from_fixtures_are_listed_and_named_in_a_finding() {
    let mut outcome = outcome(
        vec![
            case(SuiteKind::Regression, "a", "outcome"),
            seeded("b", "crate"),
            seeded("c", "docs"),
        ],
        Vec::new(),
    );
    outcome.check = true;
    outcome.drove = None;

    let mut out = Vec::new();
    report(&outcome, &mut out).expect("written");
    let envelope: Value = serde_json::from_slice(&out).expect("json");

    assert_eq!(
        (&envelope["data"]["scaffolded"], &envelope["findings"]),
        (
            &json!([
                {"case": "plan/regression/b.json", "fixture": "crate"},
                {"case": "plan/regression/c.json", "fixture": "docs"}
            ]),
            &json!([{
                "key": "eval/uplift/scaffolded",
                "message": "a run seeds 2 cases from fixtures with scaffold scripts that run \
                            outside the eval sandbox with your rights (plan/regression/b.json, \
                            plan/regression/c.json); review those cases and fixtures before \
                            running them with --allow-scaffolds"
            }])
        )
    );
}

#[test]
fn a_run_names_the_target_its_copies_got_the_binary_under() {
    let mut placed = outcome(Vec::new(), Vec::new());
    placed.anneal = Some(PlacedAnneal::Target("x86_64-unknown-linux-musl"));
    let mut unknown = outcome(Vec::new(), Vec::new());
    unknown.anneal = Some(PlacedAnneal::UnknownHost);

    let envelopes = [placed, unknown].map(|outcome| {
        let mut out = Vec::new();
        report(&outcome, &mut out).expect("written");
        serde_json::from_slice::<Value>(&out).expect("json")
    });

    assert_eq!(
        envelopes
            .iter()
            .map(|envelope| (&envelope["data"]["libexec_target"], &envelope["findings"]))
            .collect::<Vec<_>>(),
        [
            (&json!("x86_64-unknown-linux-musl"), &json!([])),
            (
                &Value::Null,
                &json!([{
                    "key": "eval/uplift/libexec",
                    "message": "no anneal binary was placed in the plugin copies: the plugin's \
                                launcher knows no target for this host, so the runs' hooks and \
                                skills found anneal only if the launcher could"
                }])
            ),
        ]
    );
}
