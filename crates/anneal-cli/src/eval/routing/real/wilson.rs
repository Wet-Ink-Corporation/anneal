//! Wilson score intervals and the Tier 2 bars: pure, no I/O (`evals/README.md`, Tier 2).
//!
//! A firing rate measured over a few runs is reported as the 95% Wilson score interval
//! rather than a bare proportion, so a perfect 3 of 3 reads as "somewhere between 0.44 and
//! 1", not "100%".

use serde::Serialize;

/// The two-sided 95% normal quantile (the 97.5th percentile of the standard normal).
pub(crate) const Z_95: f64 = 1.959_963_984_540_054;

/// A closed interval within `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Interval {
    lower: f64,
    upper: f64,
}

impl Interval {
    /// The lower bound.
    pub(crate) const fn lower(self) -> f64 {
        self.lower
    }

    /// The upper bound.
    pub(crate) const fn upper(self) -> f64 {
        self.upper
    }
}

/// The 95% Wilson score interval for `successes` of `trials`, or `None` when there are no
/// trials or more successes than trials.
pub(crate) fn wilson(successes: u32, trials: u32) -> Option<Interval> {
    if trials == 0 || successes > trials {
        return None;
    }
    let n = f64::from(trials);
    let p = f64::from(successes) / n;
    let z2 = Z_95 * Z_95;
    let denominator = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denominator;
    let margin = Z_95 * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    // Rounding can leave 0 of n a hair below zero, or n of n a hair above one.
    Some(Interval {
        lower: (centre - margin).clamp(0.0, 1.0),
        upper: (centre + margin).clamp(0.0, 1.0),
    })
}

/// Which bar a firing rate is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Bar {
    /// The skill should fire: the lower bound must be at least [`SHOULD_FIRE_MIN`].
    ShouldFire,
    /// The skill should not fire: the upper bound must be at most [`SHOULD_NOT_MAX`].
    ShouldNot,
}

/// The least lower bound a should-fire rate may have.
pub(crate) const SHOULD_FIRE_MIN: f64 = 0.70;
/// The greatest upper bound a should-not rate may have.
pub(crate) const SHOULD_NOT_MAX: f64 = 0.20;

/// Where an interval lies against a bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Position {
    /// The whole interval meets the bar.
    Met,
    /// The whole interval misses it.
    Missed,
    /// The interval holds the threshold: more runs could settle it either way.
    Straddles,
}

impl Bar {
    /// The bar's threshold.
    pub(crate) const fn threshold(self) -> f64 {
        match self {
            Self::ShouldFire => SHOULD_FIRE_MIN,
            Self::ShouldNot => SHOULD_NOT_MAX,
        }
    }

    /// Where `interval` lies against this bar. A bound exactly at the threshold meets it.
    pub(crate) fn position(self, interval: Interval) -> Position {
        let threshold = self.threshold();
        match self {
            Self::ShouldFire if interval.lower >= threshold => Position::Met,
            Self::ShouldFire if interval.upper < threshold => Position::Missed,
            Self::ShouldNot if interval.upper <= threshold => Position::Met,
            Self::ShouldNot if interval.lower > threshold => Position::Missed,
            Self::ShouldFire | Self::ShouldNot => Position::Straddles,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Bar, Interval, Position, wilson};

    /// Reference values: the Wilson score interval at z = 1.959963984540054, computed
    /// independently (Python, `math.sqrt`) to ten places.
    const TOLERANCE: f64 = 1e-9;

    fn close(actual: Interval, lower: f64, upper: f64) -> bool {
        (actual.lower() - lower).abs() < TOLERANCE && (actual.upper() - upper).abs() < TOLERANCE
    }

    #[test]
    fn twenty_one_of_twenty_seven_has_a_lower_bound_of_about_0_59() {
        let interval = wilson(21, 27).expect("27 trials");

        assert!(
            close(interval, 0.592_430_711_3, 0.893_927_605_5),
            "{interval:?}"
        );
    }

    #[test]
    fn small_and_perfect_counts_match_the_reference_values() {
        let cases = [
            (0, 3, 0.0, 0.561_497_031_8),
            (3, 3, 0.438_502_968_2, 1.0),
            (2, 3, 0.207_659_600_8, 0.938_508_055_3),
            (5, 10, 0.236_593_090_5, 0.763_406_909_5),
            (9, 9, 0.700_854_951_6, 1.0),
            (11, 12, 0.646_120_088_9, 0.985_134_905_6),
            (0, 12, 0.0, 0.242_494_006_7),
            (0, 16, 0.0, 0.193_607_680_5),
            (0, 162, 0.0, 0.023_163_440_8),
        ];

        for (successes, trials, lower, upper) in cases {
            let interval = wilson(successes, trials).expect("trials");
            assert!(
                close(interval, lower, upper),
                "{successes}/{trials}: {interval:?}"
            );
        }
    }

    #[test]
    fn no_trials_or_more_successes_than_trials_has_no_interval() {
        assert_eq!((wilson(0, 0), wilson(4, 3)), (None, None));
    }

    #[test]
    fn a_should_fire_rate_meets_misses_or_straddles_0_70() {
        let positions = [(9, 9), (0, 3), (3, 3), (11, 12)].map(|(successes, trials)| {
            Bar::ShouldFire.position(wilson(successes, trials).expect("trials"))
        });

        assert_eq!(
            positions,
            [
                Position::Met,
                Position::Missed,
                Position::Straddles,
                Position::Straddles
            ],
            "9/9 lower 0.7009; 0/3 upper 0.561; 3/3 lower 0.439; 11/12 lower 0.646"
        );
    }

    #[test]
    fn a_should_not_rate_meets_misses_or_straddles_0_20() {
        let positions = [(0, 16), (5, 10), (0, 12)].map(|(successes, trials)| {
            Bar::ShouldNot.position(wilson(successes, trials).expect("trials"))
        });

        assert_eq!(
            positions,
            [Position::Met, Position::Missed, Position::Straddles],
            "0/16 upper 0.194; 5/10 lower 0.237; 0/12 upper 0.242"
        );
    }

    #[test]
    fn a_bound_exactly_at_the_threshold_meets_the_bar() {
        let at = |lower, upper| Interval { lower, upper };

        assert_eq!(
            (
                Bar::ShouldFire.position(at(0.70, 0.9)),
                Bar::ShouldNot.position(at(0.0, 0.20))
            ),
            (Position::Met, Position::Met)
        );
    }
}
