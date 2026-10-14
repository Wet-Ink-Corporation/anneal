//! The calibration maths, on labelled samples: the Brier score, reliability bins, the expected
//! cost of a threshold and the threshold that minimises it.
//!
//! Counts are `u32`, saturating: the decision log holds at most 80 MiB of rows of a few hundred
//! bytes each, far below `u32::MAX`, and `f64::from(u32)` is exact (H-11).

use serde::Serialize;

/// How many equal-width reliability bins cover 0..=1.
pub(crate) const BINS: u32 = 10;

/// What a false accept costs: the error the point exists to prevent gets through.
pub(crate) const FALSE_ACCEPT_COST: u32 = 5;

/// What a needless intervention costs: an ask, block or hint that was not needed.
pub(crate) const NEEDLESS_COST: u32 = 1;

/// One answered decision with a usable label: the probability the thresholds applied to, and
/// whether the predicted event happened (noul and score: `yes`; choice: the top option).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sample {
    probability: f64,
    positive: bool,
}

impl Sample {
    /// A sample; `probability` is within 0..=1 (the log reader checks it).
    pub(crate) const fn new(probability: f64, positive: bool) -> Self {
        Self {
            probability,
            positive,
        }
    }
}

/// One reliability bin: `[lower, upper)`, the last closed at 1.0.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Bin {
    lower: f64,
    upper: f64,
    count: u32,
    /// The mean probability in the bin; `None` when it is empty.
    mean_probability: Option<f64>,
    /// The fraction of positives in the bin; `None` when it is empty.
    observed_frequency: Option<f64>,
}

/// Which error of a threshold is the false accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FalseAccept {
    /// A positive below the threshold: the point let it through (D1 destructive, D2 one-way
    /// door, D5 defect risk, D7 a decision for the human).
    BelowThreshold,
    /// A negative at or above the threshold: the point acted on a wrong answer (D3 a wrong
    /// phase hint, D4 passing an incomplete section, D6 surfacing the wrong ruling).
    AtOrAboveThreshold,
}

/// The cost model of one point: which side is the false accept, and what each error costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Costs {
    false_accept: FalseAccept,
    false_accept_cost: u32,
    needless_cost: u32,
}

impl Costs {
    /// The costs with `false_accept` as the false accept, at [`FALSE_ACCEPT_COST`] and
    /// [`NEEDLESS_COST`].
    pub(crate) const fn new(false_accept: FalseAccept) -> Self {
        Self {
            false_accept,
            false_accept_cost: FALSE_ACCEPT_COST,
            needless_cost: NEEDLESS_COST,
        }
    }

    /// The total cost of a threshold that leaves `below` under it and `above` at or over it.
    fn total(self, below: Tally, above: Tally) -> u32 {
        let (false_accepts, needless) = match self.false_accept {
            FalseAccept::BelowThreshold => (below.positive, above.negative),
            FalseAccept::AtOrAboveThreshold => (above.negative, below.positive),
        };
        false_accepts
            .saturating_mul(self.false_accept_cost)
            .saturating_add(needless.saturating_mul(self.needless_cost))
    }
}

/// The threshold that minimises the total cost, and that cost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Proposal {
    /// The threshold (`at_least`).
    pub(crate) threshold: f64,
    /// The total cost on the labelled set at that threshold.
    pub(crate) total_cost: u32,
}

/// Positives and negatives in a set of samples.
#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    positive: u32,
    negative: u32,
}

impl Tally {
    fn of<'a>(samples: impl IntoIterator<Item = &'a Sample>) -> Self {
        samples.into_iter().fold(Self::default(), |tally, sample| {
            tally.plus(Self {
                positive: u32::from(sample.positive),
                negative: u32::from(!sample.positive),
            })
        })
    }

    const fn plus(self, other: Self) -> Self {
        Self {
            positive: self.positive.saturating_add(other.positive),
            negative: self.negative.saturating_add(other.negative),
        }
    }

    const fn minus(self, other: Self) -> Self {
        Self {
            positive: self.positive.saturating_sub(other.positive),
            negative: self.negative.saturating_sub(other.negative),
        }
    }
}

/// `len` as a count, saturating (see the module documentation).
pub(crate) fn count(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// The mean of `(p - o)^2`, with `o` 1 for a positive and 0 otherwise; `None` for no samples.
pub(crate) fn brier(samples: &[Sample]) -> Option<f64> {
    let n = count(samples.len());
    if n == 0 {
        return None;
    }
    let sum: f64 = samples
        .iter()
        .map(|sample| (sample.probability - f64::from(u8::from(sample.positive))).powi(2))
        .sum();
    Some(sum / f64::from(n))
}

/// The [`BINS`] reliability bins, `[k/10, (k+1)/10)` with 1.0 in the last.
pub(crate) fn bins(samples: &[Sample]) -> Vec<Bin> {
    (0..BINS)
        .map(|k| {
            let lower = f64::from(k) / f64::from(BINS);
            let upper = f64::from(k.saturating_add(1)) / f64::from(BINS);
            let last = k.saturating_add(1) == BINS;
            let inside = samples.iter().filter(|sample| {
                let p = sample.probability;
                p >= lower && (p < upper || (last && p <= upper))
            });
            let (tally, probability_sum) = inside
                .fold((Tally::default(), 0.0), |(tally, sum), sample| {
                    (tally.plus(Tally::of([sample])), sum + sample.probability)
                });
            let n = tally.positive.saturating_add(tally.negative);
            let mean = |sum: f64| (n > 0).then(|| sum / f64::from(n));
            Bin {
                lower,
                upper,
                count: n,
                mean_probability: mean(probability_sum),
                observed_frequency: mean(f64::from(tally.positive)),
            }
        })
        .collect()
}

/// The total cost of `threshold` on `samples`.
pub(crate) fn total_cost(samples: &[Sample], costs: Costs, threshold: f64) -> u32 {
    let below = Tally::of(samples.iter().filter(|s| s.probability < threshold));
    let above = Tally::of(samples.iter().filter(|s| s.probability >= threshold));
    costs.total(below, above)
}

/// The cost of `threshold` per labelled decision; `None` for no samples.
pub(crate) fn expected_cost(samples: &[Sample], costs: Costs, threshold: f64) -> Option<f64> {
    let n = count(samples.len());
    (n > 0).then(|| f64::from(total_cost(samples, costs, threshold)) / f64::from(n))
}

/// The threshold to propose in place of `current`: the cheapest candidate, but only when it
/// costs strictly less than `current`; otherwise `current` itself, with its cost. `None` only
/// for no samples.
///
/// The candidates stay strictly below `ceiling` (the next band up), so the bands stay strictly
/// descending: each distinct observed probability, and the midpoint between the highest
/// observed probability under the ceiling and the ceiling (1.0 without one), which acts on
/// none of them. Among candidates a tie goes to the higher threshold, which acts on fewer
/// decisions.
pub(crate) fn propose(
    samples: &[Sample],
    costs: Costs,
    current: f64,
    ceiling: Option<f64>,
) -> Option<Proposal> {
    if samples.is_empty() {
        return None;
    }
    let current = Proposal {
        threshold: current,
        total_cost: total_cost(samples, costs, current),
    };
    let candidates = [
        cheapest_observed(samples, costs, ceiling),
        above_all(samples, costs, ceiling),
    ];
    let best = candidates.into_iter().flatten().reduce(|best, next| {
        let cheaper = next.total_cost < best.total_cost;
        let tie_and_higher = next.total_cost == best.total_cost && next.threshold > best.threshold;
        if cheaper || tie_and_higher {
            next
        } else {
            best
        }
    });
    Some(
        best.filter(|best| best.total_cost < current.total_cost)
            .unwrap_or(current),
    )
}

/// The candidate that acts on no observed probability under the ceiling.
fn above_all(samples: &[Sample], costs: Costs, ceiling: Option<f64>) -> Option<Proposal> {
    let upper = ceiling.unwrap_or(1.0);
    let highest = samples
        .iter()
        .map(|sample| sample.probability)
        .filter(|probability| *probability < upper)
        .max_by(f64::total_cmp)?;
    let threshold = f64::midpoint(highest, upper);
    (highest < threshold && threshold < upper).then(|| Proposal {
        threshold,
        total_cost: total_cost(samples, costs, threshold),
    })
}

/// The observed probability under the ceiling with the least total cost, ties to the higher.
fn cheapest_observed(samples: &[Sample], costs: Costs, ceiling: Option<f64>) -> Option<Proposal> {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.probability.total_cmp(&b.probability));
    let all = Tally::of(&sorted);
    let mut below = Tally::default();
    let mut best: Option<Proposal> = None;
    let mut rest = sorted.as_slice();
    while let Some(first) = rest.first() {
        let threshold = first.probability;
        if ceiling.is_none_or(|ceiling| threshold < ceiling) {
            let total_cost = costs.total(below, all.minus(below));
            // `<=`: thresholds rise, so a later tie is the higher threshold.
            if best.is_none_or(|best| total_cost <= best.total_cost) {
                best = Some(Proposal {
                    threshold,
                    total_cost,
                });
            }
        }
        let same = rest
            .iter()
            .take_while(|sample| sample.probability.total_cmp(&threshold).is_eq())
            .count();
        let Some((group, tail)) = rest.split_at_checked(same) else {
            break;
        };
        below = below.plus(Tally::of(group));
        rest = tail;
    }
    best
}

#[cfg(test)]
mod tests;
