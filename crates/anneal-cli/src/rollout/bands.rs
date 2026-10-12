//! Rollout bands: per metric, the thresholds at which a rollout holds and rolls back, and the
//! decision a set of metrics gets from them. Pure: the file is read by `rollout`.
//!
//! A threshold belongs to the worse band. When lower is better, a value below `hold` is go,
//! from `hold` up to (not including) `rollback` it holds, and from `rollback` up it rolls back;
//! when higher is better the comparisons mirror (above `hold` go, down to `rollback` exclusive
//! hold, at or below `rollback` roll back). A banded metric missing from the input holds: no
//! evidence is not a go. The worst band of any metric is the decision.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::json_file::UniqueMap;

/// The most metrics a bands file may hold (H-13).
pub const MAX_METRICS: usize = 256;

/// Which direction of a metric is the good one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Better {
    /// Lower is better: error rates, latency.
    Lower,
    /// Higher is better: success rates, availability.
    Higher,
}

/// A metric's band, from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Rating {
    /// Within bounds: carry on.
    Go,
    /// Between the hold and rollback thresholds, or not measured: stop widening.
    Hold,
    /// At or past the rollback threshold: roll back.
    Rollback,
}

/// Why a band or a bands file is refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BandError {
    /// A threshold is not a finite number.
    #[error("a threshold must be a finite number")]
    NotFinite,
    /// The hold threshold is past the rollback threshold.
    #[error(
        "the hold threshold is past the rollback threshold (better: {better:?}); hold must be \
         reached first"
    )]
    Order {
        /// The band's direction.
        better: Better,
    },
    /// The file names no metric, or more than [`MAX_METRICS`].
    #[error("a bands file holds 1 to {MAX_METRICS} metrics, found {count}")]
    Count {
        /// How many it holds.
        count: usize,
    },
    /// One metric's band is invalid.
    #[error("metric {metric:?}")]
    Metric {
        /// The metric's name.
        metric: String,
        /// What is wrong with it.
        #[source]
        source: Box<Self>,
    },
}

/// A validated band: finite thresholds, hold reached no later than rollback.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Band {
    better: Better,
    hold: f64,
    rollback: f64,
}

impl Band {
    /// A band for a metric whose good direction is `better`.
    ///
    /// # Errors
    /// [`BandError::NotFinite`] for a threshold that is not finite, [`BandError::Order`] when
    /// `hold` is past `rollback` in the bad direction (equal thresholds leave no hold band).
    pub(crate) fn new(better: Better, hold: f64, rollback: f64) -> Result<Self, BandError> {
        if !hold.is_finite() || !rollback.is_finite() {
            return Err(BandError::NotFinite);
        }
        let ordered = match better {
            Better::Lower => hold <= rollback,
            Better::Higher => hold >= rollback,
        };
        if ordered {
            Ok(Self {
                better,
                hold,
                rollback,
            })
        } else {
            Err(BandError::Order { better })
        }
    }

    /// Which direction of the metric is the good one.
    pub(crate) const fn better(&self) -> Better {
        self.better
    }

    /// The threshold from which the rollout holds.
    pub(crate) const fn hold(&self) -> f64 {
        self.hold
    }

    /// The threshold from which the rollout rolls back.
    pub(crate) const fn rollback(&self) -> f64 {
        self.rollback
    }

    /// The band `value` falls in; a threshold belongs to the worse band.
    pub(crate) fn rate(&self, value: f64) -> Rating {
        let (rolls_back, holds) = match self.better {
            Better::Lower => (value >= self.rollback, value >= self.hold),
            Better::Higher => (value <= self.rollback, value <= self.hold),
        };
        if rolls_back {
            Rating::Rollback
        } else if holds {
            Rating::Hold
        } else {
            Rating::Go
        }
    }
}

/// A bands file as written: `{"metrics": {"<name>": {"better", "hold", "rollback"}}}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BandsDto {
    /// A metric named twice does not parse.
    metrics: UniqueMap<BandDto>,
}

/// One metric's band as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BandDto {
    better: Better,
    hold: f64,
    rollback: f64,
}

/// Validated bands, by metric name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bands {
    metrics: BTreeMap<String, Band>,
}

impl TryFrom<BandsDto> for Bands {
    type Error = BandError;

    fn try_from(dto: BandsDto) -> Result<Self, Self::Error> {
        let metrics = dto.metrics.into_inner();
        let count = metrics.len();
        if count == 0 || count > MAX_METRICS {
            return Err(BandError::Count { count });
        }
        let metrics = metrics
            .into_iter()
            .map(
                |(metric, band)| match Band::new(band.better, band.hold, band.rollback) {
                    Ok(band) => Ok((metric, band)),
                    Err(error) => Err(BandError::Metric {
                        metric,
                        source: Box::new(error),
                    }),
                },
            )
            .collect::<Result<_, _>>()?;
        Ok(Self { metrics })
    }
}

/// One banded metric as judged.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Rated<'a> {
    /// The metric's name.
    pub(crate) metric: &'a str,
    /// Its value, or `None` when the input did not give it.
    pub(crate) value: Option<f64>,
    /// Its band.
    pub(crate) rating: Rating,
    /// The band it was judged against.
    #[serde(flatten)]
    pub(crate) band: Band,
}

/// The decision for a set of metrics.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decision<'a> {
    /// The worst band of any metric.
    pub(crate) rating: Rating,
    /// Every banded metric, by name.
    rated: Vec<Rated<'a>>,
    /// Input metrics no band names, by name; they do not change the decision.
    pub(crate) unbanded: Vec<&'a str>,
}

impl<'a> Decision<'a> {
    /// Judges `metrics` against `bands`.
    pub(crate) fn of(bands: &'a Bands, metrics: &'a BTreeMap<String, f64>) -> Self {
        let rated: Vec<Rated<'a>> = bands
            .metrics
            .iter()
            .map(|(metric, band)| {
                let value = metrics.get(metric).copied();
                Rated {
                    metric,
                    value,
                    rating: value.map_or(Rating::Hold, |value| band.rate(value)),
                    band: *band,
                }
            })
            .collect();
        let rating = rated
            .iter()
            .map(|rated| rated.rating)
            .max()
            .unwrap_or(Rating::Hold);
        let unbanded = metrics
            .keys()
            .filter(|metric| !bands.metrics.contains_key(*metric))
            .map(String::as_str)
            .collect();
        Self {
            rating,
            rated,
            unbanded,
        }
    }

    /// Every banded metric as judged, by name.
    pub(crate) fn rated(&self) -> impl Iterator<Item = &Rated<'a>> + '_ {
        self.rated.iter()
    }
}

#[cfg(test)]
mod tests;
