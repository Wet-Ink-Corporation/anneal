//! Whether a candidate's run times differ from a baseline's beyond run-to-run noise. Pure.
//!
//! The statistic is Welch's t interval on the difference of means (candidate minus baseline,
//! in seconds) at 95%: the half-width is `t * sqrt(var_c / n_c + var_b / n_b)`, with `t` the
//! 97.5th percentile of Student's t at the Welch–Satterthwaite degrees of freedom, read from a
//! table, rounded up, at the largest tabled df not above it (t falls as df grows, so the
//! interval is never narrower than the exact one). The change is faster only when the whole interval is below zero; an interval
//! holding zero is neutral, and one above zero is slower. Only faster keeps the change.

use serde::Serialize;

/// The most runs one sample may hold (H-13).
pub const MAX_RUNS: usize = 100;

/// The fewest runs one sample may hold: one run has no variance.
pub const MIN_RUNS: usize = 2;

/// The largest run time a sample may hold, in seconds (a day): beyond any `--timeout`.
const MAX_SECS: f64 = 86_400.0;

/// The confidence of the interval.
pub(crate) const CONFIDENCE: f64 = 0.95;

/// Student's t at 97.5% for 1 degree of freedom, the widest tabled value, used below 1 too.
const T_975_DF_1: f64 = 12.707;

/// Student's t at 97.5% (two-sided 95%) by degrees of freedom, ascending, each rounded up to
/// three decimals so no entry is below the exact quantile (checked against it by
/// `every_tabled_t_is_at_or_above_the_exact_one_and_within_0_001_of_it`).
const T_975: [(u32, f64); 36] = [
    (1, T_975_DF_1),
    (2, 4.303),
    (3, 3.183),
    (4, 2.777),
    (5, 2.571),
    (6, 2.447),
    (7, 2.365),
    (8, 2.307),
    (9, 2.263),
    (10, 2.229),
    (11, 2.201),
    (12, 2.179),
    (13, 2.161),
    (14, 2.145),
    (15, 2.132),
    (16, 2.120),
    (17, 2.110),
    (18, 2.101),
    (19, 2.094),
    (20, 2.086),
    (21, 2.080),
    (22, 2.074),
    (23, 2.069),
    (24, 2.064),
    (25, 2.060),
    (26, 2.056),
    (27, 2.052),
    (28, 2.049),
    (29, 2.046),
    (30, 2.043),
    (40, 2.022),
    (50, 2.009),
    (60, 2.001),
    (80, 1.991),
    (100, 1.984),
    (120, 1.980),
];

/// Why a set of run times is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SamplesError {
    /// Fewer than [`MIN_RUNS`] or more than [`MAX_RUNS`] runs.
    #[error("a sample holds {MIN_RUNS} to {MAX_RUNS} runs, found {count}")]
    Count {
        /// How many it holds.
        count: usize,
    },
    /// A run time that is negative, not finite, or over a day.
    #[error("run {index} is not a time in seconds from 0 to 86400")]
    Value {
        /// Its index, from 0.
        index: usize,
    },
}

/// Validated run times in seconds: [`MIN_RUNS`] to [`MAX_RUNS`] of them, each finite, from 0
/// to a day.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Samples {
    secs: Vec<f64>,
    /// `secs.len()`, kept as a float for the arithmetic (at most [`MAX_RUNS`], so exact).
    n: f64,
}

impl Samples {
    /// Validates `secs`.
    ///
    /// # Errors
    /// [`SamplesError::Count`] for too few or too many runs, [`SamplesError::Value`] for the
    /// first time out of range.
    pub(crate) fn new(secs: Vec<f64>) -> Result<Self, SamplesError> {
        let count = secs.len();
        let n = u32::try_from(count)
            .ok()
            .filter(|_| (MIN_RUNS..=MAX_RUNS).contains(&count))
            .ok_or(SamplesError::Count { count })?;
        if let Some(index) = secs
            .iter()
            .position(|secs| !(secs.is_finite() && (0.0..=MAX_SECS).contains(secs)))
        {
            return Err(SamplesError::Value { index });
        }
        Ok(Self {
            secs,
            n: f64::from(n),
        })
    }

    /// The run times, in seconds, in run order.
    pub(crate) fn secs(&self) -> &[f64] {
        &self.secs
    }

    /// The number of runs.
    pub(crate) fn len(&self) -> usize {
        self.secs.len()
    }

    /// The mean run time.
    pub(crate) fn mean(&self) -> f64 {
        self.secs.iter().sum::<f64>() / self.n
    }

    /// The sample variance (divided by n - 1).
    fn variance(&self) -> f64 {
        let mean = self.mean();
        let squares: f64 = self.secs.iter().map(|secs| (secs - mean).powi(2)).sum();
        squares / (self.n - 1.0)
    }
}

/// What the interval says about the candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    /// The whole interval is below zero: faster beyond the noise.
    Faster,
    /// The interval holds zero: no difference the runs can tell from noise.
    Neutral,
    /// The whole interval is above zero: slower beyond the noise.
    Slower,
}

impl Verdict {
    /// Whether the change is kept: only when it is faster.
    pub(crate) const fn keeps(self) -> bool {
        matches!(self, Self::Faster)
    }
}

/// A candidate compared with a baseline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Comparison {
    /// The candidate's mean minus the baseline's, in seconds.
    pub(crate) delta: f64,
    /// The interval's lower end.
    pub(crate) low: f64,
    /// The interval's upper end.
    pub(crate) high: f64,
    /// The Welch–Satterthwaite degrees of freedom.
    pub(crate) df: f64,
    /// The t value used.
    pub(crate) t: f64,
    /// What the interval says.
    pub(crate) verdict: Verdict,
}

impl Comparison {
    /// Compares `candidate` with `baseline`.
    pub(crate) fn of(baseline: &Samples, candidate: &Samples) -> Self {
        let delta = candidate.mean() - baseline.mean();
        let a = candidate.variance() / candidate.n;
        let b = baseline.variance() / baseline.n;
        let se2 = a + b;
        let spread = a.powi(2) / (candidate.n - 1.0) + b.powi(2) / (baseline.n - 1.0);
        let df = if spread > 0.0 {
            se2.powi(2) / spread
        } else {
            // No variance on either side: the pooled df; the half-width is 0 whatever t is.
            candidate.n + baseline.n - 2.0
        };
        let t = t_975(df);
        let half = t * se2.sqrt();
        let (low, high) = (delta - half, delta + half);
        let verdict = if high < 0.0 {
            Verdict::Faster
        } else if low > 0.0 {
            Verdict::Slower
        } else {
            Verdict::Neutral
        };
        Self {
            delta,
            low,
            high,
            df,
            t,
            verdict,
        }
    }
}

/// Student's t at 97.5% for `df` degrees of freedom: the tabled value at the largest tabled df
/// not above `df` (the first row below 1).
pub(crate) fn t_975(df: f64) -> f64 {
    let mut t = T_975_DF_1;
    for (tabled, value) in T_975 {
        if f64::from(tabled) > df {
            break;
        }
        t = value;
    }
    t
}

#[cfg(test)]
mod tests;
