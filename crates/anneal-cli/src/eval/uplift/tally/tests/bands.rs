//! Guard bands and the confirmation rule, through [`tally`].

use std::collections::BTreeMap;

use super::{ROUND, arms, case, fail, pass};
use crate::eval::uplift::Trial;
use crate::eval::uplift::case::Behaviour;
use crate::eval::uplift::runs::Runs;
use crate::eval::uplift::suites::SuiteKind;
use crate::eval::uplift::tally::{GuardBand, Rule, Scoring, UpliftBar, tally};

/// `passed` passing runs, then failing ones up to `runs`.
fn passing(passed: usize, runs: usize) -> Vec<Trial> {
    (0..runs)
        .map(|i| if i < passed { pass() } else { fail() })
        .collect()
}

#[test]
fn a_guard_that_slips_one_of_three_runs_is_watched_and_its_skill_still_passes() {
    let cases = [
        case("plan", SuiteKind::Regression, "g", Behaviour::Guard),
        case("plan", SuiteKind::Capability, "c", Behaviour::Outcome),
    ];
    let runs = BTreeMap::from([
        (cases[0].name(), arms(passing(2, 3), passing(0, 3))),
        (cases[1].name(), arms(passing(3, 3), passing(0, 3))),
    ]);

    let tallied = tally(&cases, &runs, ROUND);

    let guard = tallied.cases.first().expect("the guard");
    assert_eq!(
        (guard.guard_band, guard.guard_held),
        (Some(GuardBand::Watch), Some(false))
    );
    let plan = tallied.skills.first().expect("plan");
    assert_eq!(
        (
            plan.uplift_met,
            plan.guards_watch.as_slice(),
            plan.guards_failed.as_slice(),
            plan.passed
        ),
        (
            UpliftBar::Met,
            ["plan/regression/g.json"].as_slice(),
            [].as_slice(),
            true
        )
    );
}

#[test]
fn a_guard_that_passes_one_of_three_runs_is_broken_and_fails_its_skill() {
    let cases = [
        case("plan", SuiteKind::Regression, "g", Behaviour::Guard),
        case("plan", SuiteKind::Capability, "c", Behaviour::Outcome),
    ];
    let runs = BTreeMap::from([
        (cases[0].name(), arms(passing(1, 3), passing(0, 3))),
        (cases[1].name(), arms(passing(3, 3), passing(0, 3))),
    ]);

    let tallied = tally(&cases, &runs, ROUND);

    let plan = tallied.skills.first().expect("plan");
    assert_eq!(
        (
            tallied.cases.first().and_then(|c| c.guard_band),
            plan.guards_watch.as_slice(),
            plan.guards_failed.as_slice(),
            plan.passed
        ),
        (
            Some(GuardBand::Broken),
            [].as_slice(),
            ["plan/regression/g.json"].as_slice(),
            false
        )
    );
}

#[test]
fn a_confirmation_computes_no_uplift_and_holds_a_guard_at_9_in_10_not_8() {
    let cases = [
        case("plan", SuiteKind::Regression, "g", Behaviour::Guard),
        case("plan", SuiteKind::Capability, "c", Behaviour::Outcome),
    ];
    let confirmation = Rule {
        runs: Runs::CONFIRMATION,
        scoring: Scoring::Confirmation,
    };
    let verdicts: Vec<_> = [9, 8]
        .into_iter()
        .map(|passed| {
            // The outcome case passes no run: it never fails a confirmation.
            let runs = BTreeMap::from([
                (cases[0].name(), arms(passing(passed, 10), Vec::new())),
                (cases[1].name(), arms(passing(0, 10), Vec::new())),
            ]);
            let tallied = tally(&cases, &runs, confirmation);
            let held = tallied.cases.first().and_then(|c| c.guard_held);
            let plan = tallied.skills.into_iter().next().expect("plan");
            (plan.uplift_met, held, plan.guards_failed, plan.passed)
        })
        .collect();

    assert_eq!(
        verdicts,
        [
            (UpliftBar::NotComputed, Some(true), Vec::new(), true),
            (
                UpliftBar::NotComputed,
                Some(false),
                vec!["plan/regression/g.json"],
                false
            ),
        ]
    );
}
