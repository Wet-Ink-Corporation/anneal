//! The decision points and the question types Clef answers.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A Clef decision point. Each has its model and thresholds in `clef-thresholds.json`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
pub enum Point {
    /// Guard: is an undecided Bash command destructive or irreversible?
    #[value(name = "D1")]
    D1,
    /// Orchestrate: is this a one-way door?
    #[value(name = "D2")]
    D2,
    /// Route: which phase and size is a prompt?
    #[value(name = "D3")]
    D3,
    /// Stop completeness: is a required section present and grounded?
    #[value(name = "D4")]
    D4,
    /// Verify: how risky is the change?
    #[value(name = "D5")]
    D5,
    /// Ruled-before: has this question been ruled on already?
    #[value(name = "D6")]
    D6,
    /// Grill: is a question a decision for the human rather than a fact?
    #[value(name = "D7")]
    D7,
}

/// A name that is not one of `D1` to `D7`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a decision point: expected D1 to D7")]
pub struct UnknownPoint;

impl Point {
    /// Every point, in order.
    pub const ALL: [Self; 7] = [
        Self::D1,
        Self::D2,
        Self::D3,
        Self::D4,
        Self::D5,
        Self::D6,
        Self::D7,
    ];

    /// The name as typed: `D1` to `D7`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::D1 => "D1",
            Self::D2 => "D2",
            Self::D3 => "D3",
            Self::D4 => "D4",
            Self::D5 => "D5",
            Self::D6 => "D6",
            Self::D7 => "D7",
        }
    }
}

impl FromStr for Point {
    type Err = UnknownPoint;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|point| point.as_str() == name)
            .ok_or(UnknownPoint)
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The kind of question asked, which decides what probability the thresholds apply to.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Question {
    /// A yes-or-no predicate; the thresholds apply to the probability of `yes`.
    Noul,
    /// One of the caller's options; the thresholds apply to the top option's probability.
    Choice,
    /// A level normalised to 0..=1; the thresholds apply to the value itself.
    Score,
}

impl Question {
    /// Every question type, in order.
    pub const ALL: [Self; 3] = [Self::Noul, Self::Choice, Self::Score];

    /// The name as typed: `noul`, `choice` or `score`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Noul => "noul",
            Self::Choice => "choice",
            Self::Score => "score",
        }
    }
}

impl fmt::Display for Question {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{Point, Question};

    #[test]
    fn points_print_and_parse_as_d1_to_d7() {
        for (index, point) in Point::ALL.iter().enumerate() {
            let expected = format!("D{}", index + 1);
            assert_eq!(point.as_str(), expected, "printed as typed");
            assert_eq!(
                expected.parse::<Point>(),
                Ok(*point),
                "{expected} parses back"
            );
        }
        assert_eq!(
            "d1".parse::<Point>(),
            Err(super::UnknownPoint),
            "only capitals"
        );
    }

    #[test]
    fn questions_print_in_lower_case() {
        let names: Vec<&str> = [Question::Noul, Question::Choice, Question::Score]
            .iter()
            .map(|question| question.as_str())
            .collect();

        assert_eq!(names, ["noul", "choice", "score"]);
    }
}
