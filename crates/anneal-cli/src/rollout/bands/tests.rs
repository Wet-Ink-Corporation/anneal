use std::collections::BTreeMap;

use super::{Band, BandError, Bands, BandsDto, Better, Decision, Rating};

/// The bands of `tests/fixtures/rollout/bands.json`: two lower-is-better metrics and one
/// higher-is-better one.
fn bands() -> Bands {
    let dto: BandsDto =
        serde_json::from_str(include_str!("../../../tests/fixtures/rollout/bands.json"))
            .expect("the fixture parses");
    Bands::try_from(dto).expect("the fixture is valid")
}

fn metrics(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect()
}

fn lower(hold: f64, rollback: f64) -> Band {
    Band::new(Better::Lower, hold, rollback).expect("a valid band")
}

fn higher(hold: f64, rollback: f64) -> Band {
    Band::new(Better::Higher, hold, rollback).expect("a valid band")
}

#[test]
fn lower_is_better_is_go_below_hold_hold_from_hold_and_rollback_from_rollback() {
    let band = lower(0.01, 0.05);

    let rated = [0.009_999, 0.01, 0.049_999, 0.05, 0.5].map(|value| band.rate(value));

    assert_eq!(
        rated,
        [
            Rating::Go,
            Rating::Hold,
            Rating::Hold,
            Rating::Rollback,
            Rating::Rollback
        ],
        "each threshold belongs to the worse band"
    );
}

#[test]
fn higher_is_better_is_go_above_hold_hold_down_to_hold_and_rollback_from_rollback() {
    let band = higher(0.999, 0.99);

    let rated = [0.999_1, 0.999, 0.990_1, 0.99, 0.5].map(|value| band.rate(value));

    assert_eq!(
        rated,
        [
            Rating::Go,
            Rating::Hold,
            Rating::Hold,
            Rating::Rollback,
            Rating::Rollback
        ],
        "each threshold belongs to the worse band"
    );
}

#[test]
fn equal_thresholds_leave_no_hold_band() {
    let band = lower(10.0, 10.0);

    assert_eq!(
        [9.0, 10.0].map(|value| band.rate(value)),
        [Rating::Go, Rating::Rollback],
        "at the shared threshold it is rollback"
    );
}

#[test]
fn every_metric_in_its_go_band_is_go() {
    let (bands, metrics) = (
        bands(),
        metrics(&[
            ("error_rate", 0.009_9),
            ("p99_ms", 299.0),
            ("success_rate", 0.999_5),
        ]),
    );

    let decision = Decision::of(&bands, &metrics);

    assert_eq!(decision.rating, Rating::Go, "{decision:?}");
}

#[test]
fn every_metric_exactly_at_its_hold_threshold_is_hold() {
    let (bands, metrics) = (
        bands(),
        metrics(&[
            ("error_rate", 0.01),
            ("p99_ms", 300.0),
            ("success_rate", 0.999),
        ]),
    );

    let decision = Decision::of(&bands, &metrics);

    assert_eq!(
        decision
            .rated()
            .map(|rated| rated.rating)
            .collect::<Vec<_>>(),
        [Rating::Hold, Rating::Hold, Rating::Hold],
        "each metric holds"
    );
    assert_eq!(decision.rating, Rating::Hold, "{decision:?}");
}

#[test]
fn every_metric_exactly_at_its_rollback_threshold_is_rollback() {
    let (bands, metrics) = (
        bands(),
        metrics(&[
            ("error_rate", 0.05),
            ("p99_ms", 500.0),
            ("success_rate", 0.99),
        ]),
    );

    let decision = Decision::of(&bands, &metrics);

    assert_eq!(
        decision
            .rated()
            .map(|rated| rated.rating)
            .collect::<Vec<_>>(),
        [Rating::Rollback, Rating::Rollback, Rating::Rollback],
        "each metric rolls back"
    );
    assert_eq!(decision.rating, Rating::Rollback, "{decision:?}");
}

#[test]
fn the_worst_band_of_any_metric_wins() {
    let (bands, metrics) = (
        bands(),
        metrics(&[
            ("error_rate", 0.0),
            ("p99_ms", 400.0),
            ("success_rate", 0.98),
        ]),
    );

    let decision = Decision::of(&bands, &metrics);

    assert_eq!(
        decision.rating,
        Rating::Rollback,
        "one rollback outweighs a go and a hold"
    );
}

#[test]
fn a_banded_metric_missing_from_the_input_holds() {
    let (bands, metrics) = (
        bands(),
        metrics(&[("error_rate", 0.0), ("success_rate", 1.0)]),
    );

    let decision = Decision::of(&bands, &metrics);

    let missing: Vec<_> = decision
        .rated()
        .filter(|rated| rated.value.is_none())
        .map(|rated| (rated.metric, rated.rating))
        .collect();
    assert_eq!(missing, [("p99_ms", Rating::Hold)], "p99_ms is missing");
    assert_eq!(decision.rating, Rating::Hold, "missing is hold, not go");
}

#[test]
fn a_metric_with_no_band_is_listed_and_does_not_change_the_decision() {
    let (bands, metrics) = (
        bands(),
        metrics(&[
            ("error_rate", 0.0),
            ("p99_ms", 1.0),
            ("success_rate", 1.0),
            ("queue_depth", 1e9),
        ]),
    );

    let decision = Decision::of(&bands, &metrics);

    assert_eq!(
        (decision.rating, decision.unbanded.as_slice()),
        (Rating::Go, ["queue_depth"].as_slice()),
        "{decision:?}"
    );
}

#[test]
fn a_hold_threshold_worse_than_the_rollback_threshold_is_refused() {
    assert_eq!(
        Band::new(Better::Lower, 0.05, 0.01),
        Err(BandError::Order {
            better: Better::Lower
        }),
        "lower is better: hold must not exceed rollback"
    );
    assert_eq!(
        Band::new(Better::Higher, 0.99, 0.999),
        Err(BandError::Order {
            better: Better::Higher
        }),
        "higher is better: hold must not be below rollback"
    );
}

#[test]
fn a_bands_file_with_no_metrics_or_too_many_is_refused() {
    let empty: BandsDto = serde_json::from_str(r#"{"metrics": {}}"#).expect("parses");
    assert!(
        matches!(Bands::try_from(empty), Err(BandError::Count { count: 0 })),
        "no metrics: nothing to judge"
    );

    let many = (0..=super::MAX_METRICS)
        .map(|i| format!(r#""m{i}": {{"better": "lower", "hold": 1, "rollback": 2}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let many: BandsDto =
        serde_json::from_str(&format!(r#"{{"metrics": {{{many}}}}}"#)).expect("parses");
    assert!(
        matches!(Bands::try_from(many), Err(BandError::Count { .. })),
        "more than MAX_METRICS"
    );
}

#[test]
fn a_bands_file_with_an_unknown_key_does_not_parse() {
    let parsed = serde_json::from_str::<BandsDto>(
        r#"{"metrics": {"m": {"better": "lower", "hold": 1, "rollback": 2, "warn": 0}}}"#,
    );

    assert!(parsed.is_err(), "a misspelt or extra key is refused");
}
