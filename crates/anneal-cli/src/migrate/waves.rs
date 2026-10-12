//! A migration's waves, folded from its ledger events, and whether a next step may be
//! recorded. Pure: the events are read by `migrate`.
//!
//! A wave is started once and done once, after its start; the contract step needs at least one
//! wave done and none open; nothing follows a contract.

use std::collections::BTreeSet;

use anneal_ledger::LedgerEvent;
use anneal_ledger::event::{MigrateContract, MigrateWave};
use serde::Serialize;

/// A step `anneal migrate` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// A wave starts.
    Start(u32),
    /// A wave is done.
    Done(u32),
    /// The old shape is removed.
    Contract,
}

impl Step {
    /// The step `event` records, if it is a migration event.
    fn of(event: &LedgerEvent) -> Option<Self> {
        match event {
            LedgerEvent::MigrateWaveStarted(MigrateWave { wave, .. }) => Some(Self::Start(*wave)),
            LedgerEvent::MigrateWaveDone(MigrateWave { wave, .. }) => Some(Self::Done(*wave)),
            LedgerEvent::MigrateContract(MigrateContract { .. }) => Some(Self::Contract),
            _ => None,
        }
    }

    /// The event recording this step of `migration`.
    pub(crate) fn event(self, migration: String) -> LedgerEvent {
        match self {
            Self::Start(wave) => LedgerEvent::MigrateWaveStarted(MigrateWave { migration, wave }),
            Self::Done(wave) => LedgerEvent::MigrateWaveDone(MigrateWave { migration, wave }),
            Self::Contract => LedgerEvent::MigrateContract(MigrateContract { migration }),
        }
    }
}

/// Why a step is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "refused", rename_all = "snake_case")]
pub(crate) enum Refusal {
    /// The contract step while waves are still open.
    WavesOpen {
        /// The open waves, ascending.
        waves: Vec<u32>,
    },
    /// The contract step before any wave is done.
    NoWaveDone,
    /// A start of a wave already started.
    AlreadyStarted {
        /// The wave.
        wave: u32,
    },
    /// A done of a wave that is not open.
    NotOpen {
        /// The wave.
        wave: u32,
    },
    /// Any step after the contract.
    Contracted,
    /// Another step of the migration was recorded between reading its state and appending;
    /// never returned by [`Waves::check`].
    Concurrent,
}

impl Refusal {
    /// The refusal's kind, as in finding keys.
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::WavesOpen { .. } => "waves-open",
            Self::NoWaveDone => "no-wave-done",
            Self::AlreadyStarted { .. } => "already-started",
            Self::NotOpen { .. } => "not-open",
            Self::Contracted => "contracted",
            Self::Concurrent => "concurrent",
        }
    }
}

/// A migration's state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Waves {
    /// Waves started and not done.
    open: BTreeSet<u32>,
    /// Waves done.
    done: BTreeSet<u32>,
    /// Whether the contract step was recorded.
    contracted: bool,
}

impl Waves {
    /// Folds a migration's events, in ledger order; other events are ignored.
    pub(crate) fn fold<'a>(events: impl IntoIterator<Item = &'a LedgerEvent>) -> Self {
        events
            .into_iter()
            .filter_map(Step::of)
            .fold(Self::default(), Self::after)
    }

    /// Whether `step` may be recorded next.
    ///
    /// # Errors
    /// The [`Refusal`] saying why not.
    pub(crate) fn check(&self, step: Step) -> Result<(), Refusal> {
        if self.contracted {
            return Err(Refusal::Contracted);
        }
        match step {
            Step::Start(wave) if self.open.contains(&wave) || self.done.contains(&wave) => {
                Err(Refusal::AlreadyStarted { wave })
            }
            Step::Done(wave) if !self.open.contains(&wave) => Err(Refusal::NotOpen { wave }),
            Step::Contract if !self.open.is_empty() => Err(Refusal::WavesOpen {
                waves: self.open.iter().copied().collect(),
            }),
            Step::Contract if self.done.is_empty() => Err(Refusal::NoWaveDone),
            Step::Start(_) | Step::Done(_) | Step::Contract => Ok(()),
        }
    }

    /// The state after `step` (one [`Waves::check`] allowed, or one already recorded).
    pub(crate) fn after(mut self, step: Step) -> Self {
        match step {
            Step::Start(wave) => {
                self.open.insert(wave);
            }
            Step::Done(wave) => {
                self.open.remove(&wave);
                self.done.insert(wave);
            }
            Step::Contract => self.contracted = true,
        }
        self
    }
}

#[cfg(test)]
mod tests;
