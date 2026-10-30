//! Outcome label rows: who gave a label (`--actor`) and through what (`--via`). `plan approve`
//! and `verdict set` take the same two flags and record them as the ledger's `by` (#62).
//!
//! Both are optional and omitted from the row when not given, so a label written without them
//! is byte for byte the row `decide label` wrote before they existed, and claims nothing about
//! its source. `calibrate` reads `via` as text, so a value a newer build adds still reads.

use anneal_ledger::event::{self, By};
use serde::Serialize;

use crate::decide::id::{DecisionId, OutcomeLabel};

/// Who gave a label, a plan approval or a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    /// A person.
    Human,
    /// A model.
    Model,
}

/// Through what a label, a plan approval or a verdict was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// A Claude Code mod's buttons (the cockpit).
    Mod,
    /// A model's Bash tool call.
    Bash,
    /// A terminal, by hand.
    Terminal,
}

impl Via {
    /// The name, as serialised.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::Bash => "bash",
            Self::Terminal => "terminal",
        }
    }
}

/// Where a label came from, as far as its caller said; each part is serialised only when known.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Source {
    /// Who gave the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) actor: Option<Actor>,
    /// Through what it was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) via: Option<Via>,
}

impl Source {
    /// The ledger's `by` for an approval or a verdict given from this source (`plan approve`,
    /// `verdict set`, #62); `None` when neither part is known, so nothing is written.
    pub(crate) fn by(self) -> Option<By> {
        By::known(
            self.actor.map(event::Actor::from),
            self.via.map(event::Via::from),
        )
    }
}

impl From<Actor> for event::Actor {
    fn from(actor: Actor) -> Self {
        match actor {
            Actor::Human => Self::Human,
            Actor::Model => Self::Model,
        }
    }
}

impl From<Via> for event::Via {
    fn from(via: Via) -> Self {
        match via {
            Via::Mod => Self::Mod,
            Via::Bash => Self::Bash,
            Via::Terminal => Self::Terminal,
        }
    }
}

/// A label row: `{"kind": "label", "id", "outcome", "ts_ms"}`, then `actor` and `via` when
/// known.
#[derive(Debug, Serialize)]
pub(crate) struct LabelRow<'a> {
    kind: &'static str,
    id: &'a DecisionId,
    outcome: &'a OutcomeLabel,
    ts_ms: u64,
    #[serde(flatten)]
    source: Source,
}

impl<'a> LabelRow<'a> {
    /// The row labelling `id` with `outcome` at `ts_ms`, from `source`.
    pub(crate) const fn new(
        id: &'a DecisionId,
        outcome: &'a OutcomeLabel,
        ts_ms: u64,
        source: Source,
    ) -> Self {
        Self {
            kind: "label",
            id,
            outcome,
            ts_ms,
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;

    use super::Via;

    #[test]
    fn each_via_is_serialised_and_parsed_by_its_name() {
        for via in Via::value_variants() {
            let serialised = serde_json::to_value(via).expect("a unit variant serialises");
            assert_eq!(serialised, serde_json::json!(via.as_str()));
            assert_eq!(Via::from_str(via.as_str(), false), Ok(*via));
        }
    }
}
