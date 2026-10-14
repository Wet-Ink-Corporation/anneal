use super::{Bin, Costs, FalseAccept, Proposal, Sample, bins, brier, expected_cost, propose};

/// The hand-computed set (docs/calibrate.md works it through):
/// (0.1, no), (0.3, yes), (0.8, yes), (0.95, no), (1.0, yes).
fn hand_set() -> Vec<Sample> {
    [
        (0.1, false),
        (0.3, true),
        (0.8, true),
        (0.95, false),
        (1.0, true),
    ]
    .into_iter()
    .map(|(probability, positive)| Sample::new(probability, positive))
    .collect()
}

/// D1's shape: letting a positive through below the threshold is the false accept.
const D1_COSTS: Costs = Costs::new(FalseAccept::BelowThreshold);

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-12
}

#[test]
fn brier_of_the_hand_set_is_0_2885() {
    // The squared errors are 0.01, 0.49, 0.04, 0.9025 and 0: a sum of 1.4425, a mean of
    // 0.2885.
    let score = brier(&hand_set()).expect("five samples");

    assert!(close(score, 0.2885), "{score}");
}

#[test]
fn brier_of_no_samples_is_none() {
    assert_eq!(brier(&[]), None, "nothing to average");
}

#[test]
fn bins_split_0_to_1_in_tenths_with_1_0_in_the_last() {
    let bins = bins(&hand_set());

    assert_eq!(bins.len(), 10, "ten equal-width bins");
    let counts: Vec<u32> = bins.iter().map(|bin: &Bin| bin.count).collect();
    assert_eq!(
        counts,
        [0, 1, 0, 1, 0, 0, 0, 0, 1, 2],
        "0.3 is in [0.3, 0.4)"
    );
    for (k, bin) in bins.iter().enumerate() {
        let k = f64::from(u32::try_from(k).expect("ten bins"));
        assert!(close(bin.lower, k / 10.0), "bin {k} lower");
        assert!(close(bin.upper, (k + 1.0) / 10.0), "bin {k} upper");
    }
}

#[test]
fn each_bin_reports_its_mean_probability_and_observed_frequency() {
    let bins = bins(&hand_set());

    let last = bins.get(9).expect("ten bins");
    assert!(
        close(last.mean_probability.expect("two"), 0.975),
        "{last:?}"
    );
    assert!(
        close(last.observed_frequency.expect("two"), 0.5),
        "{last:?}"
    );
    let third = bins.get(3).expect("ten bins");
    assert!(
        close(third.observed_frequency.expect("one"), 1.0),
        "{third:?}"
    );
    let first = bins.get(1).expect("ten bins");
    assert!(
        close(first.observed_frequency.expect("one"), 0.0),
        "{first:?}"
    );
    let empty = bins.first().expect("ten bins");
    assert_eq!(
        (empty.mean_probability, empty.observed_frequency),
        (None, None),
        "an empty bin has neither"
    );
}

#[test]
fn expected_cost_weighs_a_false_accept_5_and_a_needless_ask_1() {
    // At 0.4: 0.3 (yes) is let through (5) and 0.95 (no) is a needless ask (1): 6 / 5.
    let cost = expected_cost(&hand_set(), D1_COSTS, 0.4).expect("five samples");

    assert!(close(cost, 1.2), "{cost}");
}

#[test]
fn the_proposal_is_the_cheapest_observed_probability_below_the_next_band() {
    // Candidates under 0.9: 0.1 costs 2 (two needless asks), 0.3 costs 1, 0.8 costs 6, the
    // current 0.4 costs 6, and above 0.8 (0.85) costs 5 + 5 + 1.
    let proposal = propose(&hand_set(), D1_COSTS, 0.4, Some(0.9));

    assert_eq!(
        proposal,
        Some(Proposal {
            threshold: 0.3,
            total_cost: 1
        })
    );
}

#[test]
fn a_false_accept_above_the_threshold_flips_which_errors_cost_5() {
    // D4's shape: acting (passing) on a negative is the false accept. At 0.95 nothing but
    // 0.95 (no, 5) and 1.0 (yes) is acted on; the misses 0.3 and 0.8 are needless (1 each).
    let costs = Costs::new(FalseAccept::AtOrAboveThreshold);

    let at_0_95 = expected_cost(&hand_set(), costs, 0.95).expect("five samples");
    let proposal = propose(&hand_set(), costs, 0.95, None);

    assert!(close(at_0_95, 7.0 / 5.0), "{at_0_95}");
    // 1.0 acts only on 1.0 (yes): misses 0.3 and 0.8 cost 1 each, total 2.
    assert_eq!(
        proposal,
        Some(Proposal {
            threshold: 1.0,
            total_cost: 2
        })
    );
}

#[test]
fn a_tie_goes_to_the_higher_threshold() {
    // At 0.2 the five negatives at 0.25 are needless asks (5); at 0.6 only the positive at 0.2
    // is let through (5); at 0.25 both happen (10). 0.2 and 0.6 tie, and the higher wins.
    let mut samples = vec![Sample::new(0.2, true), Sample::new(0.6, true)];
    samples.extend([Sample::new(0.25, false); 5]);

    // The current 0.25 costs 10; above every sample (0.8) both positives are let through.
    let proposal = propose(&samples, D1_COSTS, 0.25, None);

    assert_eq!(
        proposal,
        Some(Proposal {
            threshold: 0.6,
            total_cost: 5
        })
    );
}

#[test]
fn no_samples_proposes_nothing_and_no_candidate_below_the_ceiling_keeps_the_current() {
    assert_eq!(propose(&[], D1_COSTS, 0.4, None), None, "no samples");
    // 0.95 is not below the next band, and no threshold under 0.9 changes anything: 0.4 stays.
    assert_eq!(
        propose(&[Sample::new(0.95, true)], D1_COSTS, 0.4, Some(0.9)),
        Some(Proposal {
            threshold: 0.4,
            total_cost: 0
        }),
    );
}

/// The review's repro: D1 decisions at 0.1, 0.15 and 0.2, all labelled no.
fn all_no_below_the_band() -> Vec<Sample> {
    [0.1, 0.15, 0.2]
        .into_iter()
        .map(|probability| Sample::new(probability, false))
        .collect()
}

#[test]
fn a_proposal_never_costs_more_than_the_current_threshold() {
    // The current 0.4 asks nothing needlessly (0); every observed probability would (1 to 3).
    let proposal = propose(&all_no_below_the_band(), D1_COSTS, 0.4, Some(0.9));

    assert_eq!(
        proposal,
        Some(Proposal {
            threshold: 0.4,
            total_cost: 0
        }),
        "a tie with the current threshold keeps it"
    );
}

#[test]
fn a_threshold_above_every_labelled_probability_is_a_candidate() {
    // From a current 0.1 (three needless asks), acting on none below the 0.9 band costs 0:
    // the candidate is halfway between the highest labelled 0.2 and the ceiling.
    let proposal =
        propose(&all_no_below_the_band(), D1_COSTS, 0.1, Some(0.9)).expect("three samples");

    assert!(close(proposal.threshold, 0.55), "{proposal:?}");
    assert_eq!(proposal.total_cost, 0);
}

/// D1 decisions all labelled no at `probability`.
fn all_no_at(probability: f64) -> Vec<Sample> {
    vec![Sample::new(probability, false); 3]
}

#[test]
fn the_midpoint_candidate_is_rounded_to_4_decimal_places() {
    // Halfway between 0.8 and 0.9 is 0.8500000000000001 in f64; proposals.json shows 0.85.
    assert_eq!(f64::midpoint(0.8, 0.9).to_string(), "0.8500000000000001");

    let proposal = propose(&all_no_at(0.8), D1_COSTS, 0.1, Some(0.9)).expect("three samples");

    assert_eq!(proposal.threshold.to_string(), "0.85", "{proposal:?}");
    assert_eq!(proposal.total_cost, 0, "acts on none of them");
}

#[test]
fn a_rounding_that_would_leave_the_gap_keeps_the_exact_midpoint() {
    // Between 0.85001 and 0.85002 the midpoint 0.850015 rounds to 0.85, below both.
    let (highest, ceiling) = (0.85001, 0.85002);

    let proposal =
        propose(&all_no_at(highest), D1_COSTS, 0.1, Some(ceiling)).expect("three samples");

    assert_eq!(
        proposal.threshold.to_bits(),
        f64::midpoint(highest, ceiling).to_bits(),
        "the exact midpoint: {proposal:?}"
    );
    assert!(
        highest < proposal.threshold && proposal.threshold < ceiling,
        "strictly between its neighbours: {proposal:?}"
    );
}
