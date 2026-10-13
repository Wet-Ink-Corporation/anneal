//! The uplift interval: pure, no I/O.
//!
//! A skill's uplift is the difference between its pass rate with the skill and without it.
//! Over a few runs a side that difference is reported as Newcombe's hybrid score interval
//! (method 10 of Newcombe 1998), built from each arm's 95% Wilson interval, so it reuses
//! Tier 2's [`wilson`] and stays within `[-1, 1]` where a Wald interval would not.

use serde::Serialize;

use crate::eval::real::wilson::wilson;

/// Passing runs out of the runs that count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Count {
    /// Runs that passed.
    pub(crate) passed: u32,
    /// Runs that count as trials.
    pub(crate) trials: u32,
}

impl Count {
    /// `passed` of `trials`.
    pub(crate) const fn new(passed: u32, trials: u32) -> Self {
        Self { passed, trials }
    }

    /// The pass rate, or `None` with no trials.
    pub(crate) fn rate(self) -> Option<f64> {
        (self.trials > 0 && self.passed <= self.trials)
            .then(|| f64::from(self.passed) / f64::from(self.trials))
    }

    /// This count with `other` added, saturating.
    pub(crate) const fn plus(self, other: Self) -> Self {
        Self {
            passed: self.passed.saturating_add(other.passed),
            trials: self.trials.saturating_add(other.trials),
        }
    }
}

/// An interval on a difference of two rates, within `[-1, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Difference {
    estimate: f64,
    lower: f64,
    upper: f64,
}

impl Difference {
    /// The point estimate: the first rate minus the second.
    pub(crate) const fn estimate(self) -> f64 {
        self.estimate
    }

    /// The lower bound.
    pub(crate) const fn lower(self) -> f64 {
        self.lower
    }

    /// The upper bound.
    pub(crate) const fn upper(self) -> f64 {
        self.upper
    }
}

/// The 95% Newcombe hybrid score interval on `with`'s pass rate minus `base`'s, or `None`
/// when either has no trials or more passes than trials.
pub(crate) fn newcombe(with: Count, base: Count) -> Option<Difference> {
    let (p1, p2) = (with.rate()?, base.rate()?);
    let (one, two) = (
        wilson(with.passed, with.trials)?,
        wilson(base.passed, base.trials)?,
    );
    let estimate = p1 - p2;
    let below = (p1 - one.lower()).hypot(two.upper() - p2);
    let above = (one.upper() - p1).hypot(p2 - two.lower());
    // Rounding can leave a bound a hair outside [-1, 1].
    Some(Difference {
        estimate,
        lower: (estimate - below).clamp(-1.0, 1.0),
        upper: (estimate + above).clamp(-1.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::{Count, Difference, newcombe};

    /// Reference values: Newcombe (1998), "Interval estimation for the difference between
    /// independent proportions", method 10, recomputed independently (Python, `math.sqrt`,
    /// z = 1.959963984540054) to ten places.
    const TOLERANCE: f64 = 1e-9;

    fn close(actual: Difference, estimate: f64, lower: f64, upper: f64) -> bool {
        (actual.estimate() - estimate).abs() < TOLERANCE
            && (actual.lower() - lower).abs() < TOLERANCE
            && (actual.upper() - upper).abs() < TOLERANCE
    }

    #[test]
    fn the_difference_interval_matches_newcombes_worked_examples() {
        let cases = [
            ((56, 70), (48, 80), 0.2, 0.052_431_472_4, 0.333_872_654_0),
            ((9, 10), (3, 10), 0.6, 0.170_522_723_9, 0.809_017_973_5),
            (
                (5, 56),
                (0, 29),
                0.089_285_714_3,
                -0.038_137_147_9,
                0.192_560_013_9,
            ),
            ((0, 10), (0, 20), 0.0, -0.161_125_158_1, 0.277_532_799_9),
            ((10, 10), (0, 20), 1.0, 0.679_086_037_1, 1.0),
        ];

        for ((s1, n1), (s2, n2), estimate, lower, upper) in cases {
            let interval = newcombe(Count::new(s1, n1), Count::new(s2, n2)).expect("trials");
            assert!(
                close(interval, estimate, lower, upper),
                "{s1}/{n1} - {s2}/{n2}: {interval:?}"
            );
        }
    }

    #[test]
    fn three_runs_a_side_bound_the_uplift_only_when_the_arms_split_completely() {
        let split = newcombe(Count::new(3, 3), Count::new(0, 3)).expect("trials");
        let tied = newcombe(Count::new(3, 3), Count::new(3, 3)).expect("trials");

        assert!(
            close(split, 1.0, 0.205_923_282_5, 1.0)
                && close(tied, 0.0, -0.561_497_031_8, 0.561_497_031_8),
            "{split:?} {tied:?}"
        );
    }

    #[test]
    fn an_arm_with_no_trials_has_no_interval() {
        assert_eq!(
            (
                newcombe(Count::new(0, 0), Count::new(1, 3)),
                newcombe(Count::new(1, 3), Count::new(0, 0)),
                newcombe(Count::new(4, 3), Count::new(1, 3)),
            ),
            (None, None, None)
        );
    }
}
