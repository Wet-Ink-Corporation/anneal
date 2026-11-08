//! A guard case's verdict, and a skill's uplift against its bar.
//!
//! In a round (both arms ran) a guard may slip one run and is then watched; two slips break
//! it. A run asked for that errored or never came counts as a slip. In a confirmation
//! (`--no-baseline`, the with arm alone) a guard holds when it passes 9 in 10 of the runs
//! asked for, and no uplift is computed.

use serde::{Serialize, Serializer};

use crate::eval::uplift::runs::Runs;

/// How a run is scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scoring {
    /// Both arms ran: guards in bands, uplift held to its bar.
    Round,
    /// Only the with arm ran (`--no-baseline`): guards held to 9 in 10; no uplift.
    Confirmation,
}

/// The rule a run is scored by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rule {
    /// The runs each case was asked for in each arm.
    pub(crate) runs: Runs,
    /// How the runs are scored.
    pub(crate) scoring: Scoring,
}

/// A guard case's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GuardBand {
    /// Every run asked for passed (or, in a confirmation, 9 in 10 did).
    Held,
    /// In a round, exactly one run asked for did not pass: not failed, to be confirmed.
    Watch,
    /// In a round, two or more runs asked for did not pass.
    Broken,
    /// In a confirmation, fewer than 9 in 10 of the runs asked for passed.
    FailedConfirmation,
}

impl GuardBand {
    /// The band of a guard that passed `passed` of the `runs` asked for in a round.
    pub(crate) fn of_round(passed: u32, runs: Runs) -> Self {
        match runs.get().saturating_sub(passed) {
            0 => Self::Held,
            1 => Self::Watch,
            _ => Self::Broken,
        }
    }

    /// The band of a guard that passed `passed` of the `runs` asked for in a confirmation:
    /// held when `passed / runs >= 0.9`, in integers.
    pub(crate) fn of_confirmation(passed: u32, runs: Runs) -> Self {
        if u64::from(passed).saturating_mul(10) >= u64::from(runs.get()).saturating_mul(9) {
            Self::Held
        } else {
            Self::FailedConfirmation
        }
    }

    /// [`Self::of_round`] or [`Self::of_confirmation`], by `rule.scoring`.
    pub(crate) fn of(passed: u32, rule: Rule) -> Self {
        match rule.scoring {
            Scoring::Round => Self::of_round(passed, rule.runs),
            Scoring::Confirmation => Self::of_confirmation(passed, rule.runs),
        }
    }

    /// Whether the band fails its skill: [`Self::Broken`] or [`Self::FailedConfirmation`].
    pub(crate) const fn fails(self) -> bool {
        matches!(self, Self::Broken | Self::FailedConfirmation)
    }
}

/// A skill's uplift against its bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpliftBar {
    /// The uplift's lower bound is at the bar or above.
    Met,
    /// The uplift's lower bound is below the bar, or an arm had no trial.
    Missed,
    /// No baseline ran (`--no-baseline`), so no uplift was computed.
    NotComputed,
}

impl UpliftBar {
    /// Whether the bar fails its skill: only [`Self::Missed`] does.
    pub(crate) const fn fails(self) -> bool {
        matches!(self, Self::Missed)
    }
}

/// Serialized as `uplift_met` always was: `true` when met, `false` when missed, and `null` when
/// not computed.
impl Serialize for UpliftBar {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Met => serializer.serialize_bool(true),
            Self::Missed => serializer.serialize_bool(false),
            Self::NotComputed => serializer.serialize_none(),
        }
    }
}

#[cfg(test)]
mod tests;
