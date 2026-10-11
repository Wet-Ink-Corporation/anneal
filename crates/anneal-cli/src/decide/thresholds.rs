//! The thresholds table, `plugin/playbooks/clef-thresholds.json`: each point's model and, per
//! question type, the bands that turn a probability into an action.
//!
//! The file is read strictly (unknown fields refused) and validated into [`Thresholds`]:
//! version 1, all seven points, every model alias in the `models` table, at most
//! [`MAX_BANDS`] bands per rule with `at_least` in 0..=1 and strictly descending, and names of
//! 1 to 64 characters from `[a-z0-9_-]`. A rule applies to a probability, never to Clef's
//! `confidence` field: the first band whose `at_least` the probability reaches gives the
//! action, else `otherwise` does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::decide::point::{Point, Question};

/// The thresholds file shipped with the plugin, built into the binary for runs outside it.
pub(crate) const BUILT_IN: &str = include_str!("../../../../plugin/playbooks/clef-thresholds.json");

/// The only format version this build reads.
const FORMAT_VERSION: u32 = 1;

/// Most bands in one rule (H-13).
pub const MAX_BANDS: usize = 8;

/// Longest name (alias, subject, action, fallback).
const MAX_NAME_BYTES: usize = 64;

/// Longest model id.
const MAX_MODEL_ID_BYTES: usize = 128;

/// Why a thresholds file was refused.
#[derive(Debug, thiserror::Error)]
pub enum ThresholdsError {
    /// Not JSON of the expected shape (unknown fields included).
    #[error("not a thresholds file")]
    Json(#[source] serde_json::Error),
    /// Another format version.
    #[error("thresholds version {found}, expected {FORMAT_VERSION}")]
    Version {
        /// The version in the file.
        found: u32,
    },
    /// A decision point has no entry.
    #[error("no thresholds for {0}")]
    MissingPoint(Point),
    /// A point names a model alias the `models` table does not have.
    #[error("{point} names model {alias}, which is not in `models`")]
    UnknownModel {
        /// The point.
        point: Point,
        /// The alias it names.
        alias: String,
    },
    /// A model id has a character a URL path segment should not (space, `?`, `#`, `%`), an
    /// empty, `.` or `..` segment, or is empty or longer than 128 bytes.
    #[error("model {alias} has an invalid id")]
    ModelId {
        /// The alias whose id is invalid.
        alias: String,
    },
    /// A name is empty, longer than 64 bytes, or not from `[a-z0-9_-]`.
    #[error("invalid name {value:?}: use 1 to 64 of a-z, 0-9, `_` and `-`")]
    Name {
        /// The name as written.
        value: String,
    },
    /// A rule's bands are not strictly descending within 0..=1, or there are more than
    /// [`MAX_BANDS`].
    #[error("{point} {question}: {reason}")]
    Bands {
        /// The point.
        point: Point,
        /// The question type.
        question: Question,
        /// What is wrong.
        reason: &'static str,
    },
}

/// The validated table: every point's model and rules, one field per point so no lookup can
/// miss.
#[derive(Debug, Clone, PartialEq)]
pub struct Thresholds {
    d1: PointRules,
    d2: PointRules,
    d3: PointRules,
    d4: PointRules,
    d5: PointRules,
    d6: PointRules,
    d7: PointRules,
}

/// One point's model, fallback and rules.
#[derive(Debug, Clone, PartialEq)]
pub struct PointRules {
    model_alias: String,
    model: String,
    fallback: String,
    rules: BTreeMap<Question, Rule>,
}

/// The bands for one question type of one point. It serialises as written in the file, so a
/// log row records the thresholds that were in force.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    subject: String,
    bands: Vec<Band>,
    otherwise: String,
}

/// From `at_least` up, `action`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Band {
    at_least: f64,
    action: String,
}

/// The action a rule gives a probability, and the `at_least` of the band that gave it (`None`
/// when no band was reached and `otherwise` applied).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Applied<'r> {
    /// The action.
    pub action: &'r str,
    /// The band's threshold, if a band applied.
    pub threshold: Option<f64>,
}

impl<'r> Applied<'r> {
    /// An action and the threshold that gave it.
    #[must_use]
    pub const fn new(action: &'r str, threshold: Option<f64>) -> Self {
        Self { action, threshold }
    }
}

impl Thresholds {
    /// Parses and validates a thresholds file.
    ///
    /// # Errors
    /// [`ThresholdsError`] naming the first problem found.
    pub fn parse(bytes: &[u8]) -> Result<Self, ThresholdsError> {
        let probe: VersionProbe = serde_json::from_slice(bytes).map_err(ThresholdsError::Json)?;
        if probe.version != FORMAT_VERSION {
            return Err(ThresholdsError::Version {
                found: probe.version,
            });
        }
        let file: FileDto = serde_json::from_slice(bytes).map_err(ThresholdsError::Json)?;
        file.validate()
    }

    /// The thresholds shipped with the plugin, as built into this binary.
    ///
    /// # Errors
    /// As [`Thresholds::parse`]; a test keeps the shipped file valid.
    pub fn built_in() -> Result<Self, ThresholdsError> {
        Self::parse(BUILT_IN.as_bytes())
    }

    /// The rules of `point`.
    #[must_use]
    pub const fn point(&self, point: Point) -> &PointRules {
        match point {
            Point::D1 => &self.d1,
            Point::D2 => &self.d2,
            Point::D3 => &self.d3,
            Point::D4 => &self.d4,
            Point::D5 => &self.d5,
            Point::D6 => &self.d6,
            Point::D7 => &self.d7,
        }
    }
}

impl PointRules {
    /// The model alias (`clef-flash`, `clef`).
    #[must_use]
    pub fn model_alias(&self) -> &str {
        &self.model_alias
    }

    /// The Workers AI model id the alias resolves to.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// What the caller does when Clef gives no answer (`ask` for D1, else `fail_open`).
    #[must_use]
    pub fn fallback(&self) -> &str {
        &self.fallback
    }

    /// The rule for `question`, if this point asks it.
    #[must_use]
    pub fn rule(&self, question: Question) -> Option<&Rule> {
        self.rules.get(&question)
    }
}

impl Rule {
    /// What the question is about (`destructive_or_irreversible`).
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The action for `probability`: the first band it reaches, else `otherwise`.
    #[must_use]
    pub fn apply(&self, probability: f64) -> Applied<'_> {
        self.bands
            .iter()
            .find(|band| probability >= band.at_least)
            .map_or_else(
                || Applied::new(&self.otherwise, None),
                |band| Applied::new(&band.action, Some(band.at_least)),
            )
    }
}

/// The version alone, read first so another version is reported as such.
#[derive(Debug, Deserialize)]
struct VersionProbe {
    version: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDto {
    #[serde(rename = "version")]
    _version: u32,
    models: BTreeMap<String, String>,
    points: BTreeMap<Point, PointDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PointDto {
    model: String,
    fallback: String,
    rules: BTreeMap<Question, RuleDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleDto {
    subject: String,
    bands: Vec<BandDto>,
    otherwise: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BandDto {
    at_least: f64,
    action: String,
}

impl FileDto {
    fn validate(self) -> Result<Thresholds, ThresholdsError> {
        for (alias, id) in &self.models {
            name(alias)?;
            if !is_model_id(id) {
                return Err(ThresholdsError::ModelId {
                    // The error owns the alias; the table is still borrowed.
                    alias: alias.clone(),
                });
            }
        }
        let mut dtos = self.points;
        let mut take = |point: Point| -> Result<PointRules, ThresholdsError> {
            let dto = dtos
                .remove(&point)
                .ok_or(ThresholdsError::MissingPoint(point))?;
            let Some(model) = self.models.get(&dto.model) else {
                return Err(ThresholdsError::UnknownModel {
                    point,
                    alias: dto.model,
                });
            };
            // The table keeps the id for this point; the models map stays shared.
            let model = model.clone();
            dto.validate(point, model)
        };
        Ok(Thresholds {
            d1: take(Point::D1)?,
            d2: take(Point::D2)?,
            d3: take(Point::D3)?,
            d4: take(Point::D4)?,
            d5: take(Point::D5)?,
            d6: take(Point::D6)?,
            d7: take(Point::D7)?,
        })
    }
}

impl PointDto {
    fn validate(self, point: Point, model: String) -> Result<PointRules, ThresholdsError> {
        let mut rules = BTreeMap::new();
        for (question, rule) in self.rules {
            rules.insert(question, rule.validate(point, question)?);
        }
        Ok(PointRules {
            model_alias: self.model,
            model,
            fallback: name(&self.fallback).map(|()| self.fallback)?,
            rules,
        })
    }
}

impl RuleDto {
    fn validate(self, point: Point, question: Question) -> Result<Rule, ThresholdsError> {
        let bands_error = |reason| ThresholdsError::Bands {
            point,
            question,
            reason,
        };
        if self.bands.len() > MAX_BANDS {
            return Err(bands_error("more than 8 bands"));
        }
        if self
            .bands
            .iter()
            .any(|band| !(0.0..=1.0).contains(&band.at_least))
        {
            return Err(bands_error("at_least must be within 0..=1"));
        }
        if !self
            .bands
            .windows(2)
            .all(|pair| matches!(pair, [higher, lower] if higher.at_least > lower.at_least))
        {
            return Err(bands_error("bands must be strictly descending"));
        }
        name(&self.subject)?;
        name(&self.otherwise)?;
        let mut bands = Vec::with_capacity(self.bands.len());
        for band in self.bands {
            name(&band.action)?;
            bands.push(Band {
                at_least: band.at_least,
                action: band.action,
            });
        }
        Ok(Rule {
            subject: self.subject,
            bands,
            otherwise: self.otherwise,
        })
    }
}

/// Checks a name: 1 to 64 of `[a-z0-9_-]`.
fn name(value: &str) -> Result<(), ThresholdsError> {
    let valid = (1..=MAX_NAME_BYTES).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
        });
    if valid {
        Ok(())
    } else {
        Err(ThresholdsError::Name {
            value: value.to_owned(),
        })
    }
}

/// A model id fit for a URL path: 1 to 128 of `[A-Za-z0-9@/._:-]`, with no empty, `.` or
/// `..` segment that would change the path it is joined to.
fn is_model_id(id: &str) -> bool {
    (1..=MAX_MODEL_ID_BYTES).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@/._:-".contains(&byte))
        && id
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."))
}

#[cfg(test)]
mod tests;
