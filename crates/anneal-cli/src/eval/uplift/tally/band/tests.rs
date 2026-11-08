use super::{GuardBand, Rule, Scoring};
use crate::eval::uplift::runs::Runs;

fn runs(n: u32) -> Runs {
    Runs::new(n).expect("3 to 20")
}

#[test]
fn a_round_of_3_holds_at_3_watches_at_2_and_breaks_at_1_and_0() {
    assert_eq!(
        [3, 2, 1, 0].map(|passed| GuardBand::of_round(passed, Runs::ROUND)),
        [
            GuardBand::Held,
            GuardBand::Watch,
            GuardBand::Broken,
            GuardBand::Broken
        ]
    );
}

#[test]
fn a_round_counts_a_missing_or_errored_run_as_a_slip() {
    assert_eq!(
        [
            GuardBand::of_round(2, runs(3)),
            GuardBand::of_round(2, runs(4))
        ],
        [GuardBand::Watch, GuardBand::Broken],
        "2 passed of 3 asked slips once; 2 of 4 asked slips twice"
    );
}

#[test]
fn a_round_of_10_holds_at_10_watches_at_9_and_breaks_at_8() {
    assert_eq!(
        [10, 9, 8].map(|passed| GuardBand::of_round(passed, runs(10))),
        [GuardBand::Held, GuardBand::Watch, GuardBand::Broken]
    );
}

#[test]
fn a_confirmation_holds_at_9_in_10_and_fails_below() {
    assert_eq!(
        [
            GuardBand::of_confirmation(10, Runs::CONFIRMATION),
            GuardBand::of_confirmation(9, Runs::CONFIRMATION),
            GuardBand::of_confirmation(8, Runs::CONFIRMATION),
            GuardBand::of_confirmation(3, Runs::ROUND),
            GuardBand::of_confirmation(2, Runs::ROUND),
        ],
        [
            GuardBand::Held,
            GuardBand::Held,
            GuardBand::FailedConfirmation,
            GuardBand::Held,
            GuardBand::FailedConfirmation,
        ]
    );
}

#[test]
fn of_picks_the_band_by_the_rule_s_scoring_and_only_broken_and_failed_confirmation_fail() {
    let round = Rule {
        runs: runs(10),
        scoring: Scoring::Round,
    };
    let confirmation = Rule {
        scoring: Scoring::Confirmation,
        ..round
    };

    assert_eq!(
        (GuardBand::of(9, round), GuardBand::of(9, confirmation)),
        (GuardBand::Watch, GuardBand::Held)
    );
    assert_eq!(
        [
            GuardBand::Held,
            GuardBand::Watch,
            GuardBand::Broken,
            GuardBand::FailedConfirmation
        ]
        .map(GuardBand::fails),
        [false, false, true, true]
    );
}

/// How bad a band is, worst last.
fn rank(band: GuardBand) -> u8 {
    match band {
        GuardBand::Held => 0,
        GuardBand::Watch => 1,
        GuardBand::Broken | GuardBand::FailedConfirmation => 2,
    }
}

#[test]
fn over_every_runs_and_passes_all_passing_holds_a_pass_never_worsens_and_watch_has_passed_2() {
    for n in Runs::MIN..=Runs::MAX {
        let asked = runs(n);
        for scoring in [Scoring::Round, Scoring::Confirmation] {
            let rule = Rule {
                runs: asked,
                scoring,
            };
            assert_eq!(GuardBand::of(n, rule), GuardBand::Held, "{n} of {n}");
            for passed in 0..n {
                let (band, better) = (GuardBand::of(passed, rule), GuardBand::of(passed + 1, rule));
                assert!(
                    rank(better) <= rank(band),
                    "{scoring:?}: {} of {n} is {better:?}, {passed} is {band:?}",
                    passed + 1
                );
                assert!(
                    band != GuardBand::Watch || passed >= 2,
                    "{scoring:?}: {passed} of {n} is watched"
                );
            }
        }
    }
}
