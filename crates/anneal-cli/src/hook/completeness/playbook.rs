//! The events the active playbook requires before a session stops: one rule table, so later
//! playbooks (#42) add rows rather than code.
//!
//! A worktree's playbook is its anneal mode (`mode::active`). Each [`Rule`] of that mode names
//! an event that must be in the ledger, tagged with the worktree, at a position after the mode
//! was entered. A rule that is not met is a nudge (context for the model), never a block.

use anneal_ledger::event::{Mode, TestOutcome, TestRun};
use anneal_ledger::{EventKind, Filter, LedgerEvent, LedgerTag, Store, StoreError};

use crate::mode::{Active, Entered};

/// How many of the newest matching events a rule looks at (H-13).
const LOOK_BACK: usize = 256;

/// When a rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum When {
    /// Whenever the mode is active.
    Always,
    /// Only when a source file changed (see `edits`).
    SourceEdited,
}

/// Which events of the rule's type count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Needs {
    /// Any event of the type.
    Any,
    /// A `test_run` that passed: outcome green and exit 0.
    GreenRun,
}

/// One requirement of a mode's playbook.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rule {
    /// The mode it belongs to.
    pub(crate) mode: Mode,
    /// When it applies.
    pub(crate) when: When,
    /// The event that satisfies it.
    pub(crate) event: EventKind,
    /// Which events of that type count.
    pub(crate) needs: Needs,
    /// The context given when it is not met: what is missing and the command that records it.
    pub(crate) nudge: &'static str,
}

/// The playbook rules of v0.1. No mode (or `off`) requires nothing.
pub(crate) const RULES: [Rule; 2] = [
    Rule {
        mode: Mode::Build,
        when: When::SourceEdited,
        event: EventKind::TestRun,
        needs: Needs::GreenRun,
        nudge: "build mode: source changed but no green test run is recorded in this worktree \
                since build mode started; run `anneal task done <id> -- <test command>`",
    },
    Rule {
        mode: Mode::Diagnose,
        when: When::Always,
        event: EventKind::ReproRed,
        needs: Needs::Any,
        nudge: "diagnose mode: no red reproduction is recorded in this worktree since diagnose \
                mode started; run `anneal repro record --id <bug> -- <command>`",
    },
];

/// True for a `test_run` that passed: outcome green and exit code 0.
pub(crate) fn is_green_run(event: &LedgerEvent) -> bool {
    matches!(
        event,
        LedgerEvent::TestRun(TestRun {
            outcome: TestOutcome::Green,
            exit: Some(0),
            ..
        })
    )
}

/// The nudges of every rule of `entered`'s mode that the ledger `store` does not meet for the
/// worktree tagged `worktree`; `edited` says whether source changed.
///
/// # Errors
/// The [`StoreError`] when the ledger cannot be read.
pub(crate) fn nudges(
    store: &Store,
    worktree: &LedgerTag,
    entered: &Entered,
    edited: bool,
) -> Result<Vec<&'static str>, StoreError> {
    let mode = match entered.mode {
        Active::Build(_) | Active::IncompleteBuild => Mode::Build,
        Active::Diagnose => Mode::Diagnose,
    };
    let mut missing = Vec::new();
    for rule in RULES.iter().filter(|rule| rule.mode == mode) {
        if rule.when == When::SourceEdited && !edited {
            continue;
        }
        let filter = Filter {
            types: vec![rule.event],
            // The filter owns its tags; one small tag per rule.
            tags: vec![worktree.clone()],
        };
        let met = store
            .tail(&filter, LOOK_BACK)?
            .iter()
            .filter(|recorded| recorded.position > entered.position)
            .any(|recorded| match rule.needs {
                Needs::Any => true,
                Needs::GreenRun => is_green_run(&recorded.event.event),
            });
        if !met {
            missing.push(rule.nudge);
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests;
