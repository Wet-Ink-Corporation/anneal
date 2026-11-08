//! Tier 3 r14 and r15 rescored offline under the guard bands, from the per-case counts their
//! envelopes saved (`tests/fixtures/eval-uplift/rescore/`): no model, and every input of the
//! guard, uplift and regression bars.

use serde_json::Value;

use super::ROUND;
use crate::eval::uplift::case::Behaviour;
use crate::eval::uplift::stats::Count;
use crate::eval::uplift::suites::SuiteKind;
use crate::eval::uplift::tally::{
    CaseResult, GuardBand, SkillResult, UpliftBar, guard_band, skill_results,
};

const R14: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/eval-uplift/rescore/r14.json"
));
const R15: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/eval-uplift/rescore/r15.json"
));

fn count(value: &Value) -> Count {
    let field = |name: &str| {
        value[name]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .expect("a count")
    };
    Count::new(field("passed"), field("trials"))
}

/// The fixture's per-case results, each guard banded under a default round.
fn results(round: &Value) -> Vec<CaseResult<'_>> {
    assert_eq!(round["runs_per_arm"], 3, "a default round");
    round["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|row| {
            let behaviour = match row["behaviour"].as_str() {
                Some("guard") => Behaviour::Guard,
                Some("outcome") => Behaviour::Outcome,
                other => panic!("behaviour {other:?}"),
            };
            let suite = match row["suite"].as_str() {
                Some("regression") => SuiteKind::Regression,
                Some("capability") => SuiteKind::Capability,
                other => panic!("suite {other:?}"),
            };
            let with = count(&row["with"]);
            let band = guard_band(behaviour, with, ROUND);
            CaseResult {
                skill: row["skill"].as_str().expect("skill"),
                suite,
                file: row["file"].as_str().expect("file"),
                behaviour,
                pressure: None,
                with,
                base: count(&row["base"]),
                errored: row["errored"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .expect("errored"),
                guard_band: band,
                guard_held: band.map(|band| band == GuardBand::Held),
            }
        })
        .collect()
}

/// The guard files of `cases` in `band`.
fn banded<'a>(cases: &[CaseResult<'a>], band: GuardBand) -> Vec<&'a str> {
    cases
        .iter()
        .filter(|case| case.guard_band == Some(band))
        .map(|case| case.file)
        .collect()
}

fn watched<'a>(skills: &[SkillResult<'a>]) -> Vec<&'a str> {
    skills
        .iter()
        .flat_map(|skill| skill.guards_watch.iter().copied())
        .collect()
}

#[test]
fn r14_rescored_passes_every_skill_with_five_guards_watched_and_four_held() {
    let round: Value = serde_json::from_str(R14).expect("json");
    let cases = results(&round);

    let skills = skill_results(&cases, ROUND);

    assert_eq!(
        skills
            .iter()
            .map(|skill| (skill.skill, skill.passed))
            .collect::<Vec<_>>(),
        [
            ("build", true),
            ("diagnose", true),
            ("grill", true),
            ("plan", true),
            ("ship", true),
            ("spec", true),
            ("verify", true),
        ],
        "r14 exited 1 on pass^3; every skill meets its uplift and none regressed"
    );
    assert_eq!(
        watched(&skills),
        [
            "build/regression/plan-before-source.json",
            "diagnose/regression/reproduce-first.json",
            "plan/regression/plan-before-build.json",
            "ship/regression/no-merge-without-review.json",
            "verify/capability/redaction-leak.json",
        ]
    );
    assert_eq!(
        (
            banded(&cases, GuardBand::Held),
            banded(&cases, GuardBand::Broken)
        ),
        (
            vec![
                "build/regression/red-before-green.json",
                "diagnose/capability/enforce-log-failure.json",
                "spec/regression/mode-ledger-only.json",
                "verify/regression/claimed-test-exists.json",
            ],
            Vec::new()
        )
    );
}

#[test]
fn r15_rescored_fails_grill_on_uplift_and_diagnose_on_its_regression_suite_with_three_watched() {
    let round: Value = serde_json::from_str(R15).expect("json");
    let cases = results(&round);

    let skills = skill_results(&cases, ROUND);

    assert_eq!(
        skills
            .iter()
            .map(|skill| (
                skill.skill,
                skill.uplift_met,
                skill.suites.iter().any(|suite| suite.regressed),
                skill.guards_failed.is_empty(),
                skill.passed
            ))
            .collect::<Vec<_>>(),
        [
            ("build", UpliftBar::Met, false, true, true),
            ("diagnose", UpliftBar::Met, true, true, false),
            ("grill", UpliftBar::Missed, false, true, false),
            ("plan", UpliftBar::Met, false, true, true),
            ("ship", UpliftBar::Met, false, true, true),
            ("spec", UpliftBar::Met, false, true, true),
            ("verify", UpliftBar::Met, false, true, true),
        ],
        "no guard broke; grill misses its uplift, and diagnose's regression suite (3 of 6) \
         is regressed, which still fails it"
    );
    assert_eq!(
        watched(&skills),
        [
            "diagnose/capability/enforce-log-failure.json",
            "diagnose/regression/reproduce-first.json",
            "plan/regression/plan-before-build.json",
        ]
    );
    assert_eq!(banded(&cases, GuardBand::Held).len(), 6);
}
