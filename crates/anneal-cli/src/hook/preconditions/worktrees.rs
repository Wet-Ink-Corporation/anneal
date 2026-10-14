//! The worktrees one hook call meets, each found and read once: git is asked once per
//! directory, and each worktree's ledger, mode and preconditions are read once, however many
//! files and readings of them the call names. Every step that waits (git, a ledger lock) runs
//! within the hook's [`Budget`]; once it is spent the edit is denied.

use std::ffi::OsString;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};

use anneal_ledger::Store;

use super::{
    BUSY_TIMEOUT, Check, Denial, JUDGE_BUDGET, Ledger, RecordError, Verdict, build, mode_denial,
    relative_in, unjudged,
};
use crate::existing_ledger;
use crate::mode::scope::{self, PathKind};
use crate::mode::{self, Active, Entered};
use crate::paths::{self, PathError};
use crate::pr_diff::Budget;

/// The worktrees met so far in one hook call, and the time left to meet more.
pub(super) struct Worktrees<L> {
    /// Reads the environment (`$ANNEAL_STORE`).
    lookup: L,
    /// What git and the ledger may still take.
    budget: Budget,
    /// Each directory git was asked from, with the index in `met` of the worktree it is in;
    /// `None` when it is in no repository.
    anchors: Vec<(PathBuf, Option<usize>)>,
    /// Each worktree met, by its root as git printed it.
    met: Vec<Worktree>,
}

/// Where the directory an edit lands in stands.
enum Place {
    /// In the worktree at this index of `met`.
    Worktree(usize),
    /// In no git repository: the edit is allowed.
    NoRepository,
    /// Its worktree cannot be found or read in time: the edit is denied.
    Denied(Verdict),
}

/// One worktree and what gates it.
struct Worktree {
    /// The top-level directory, as `git rev-parse --show-toplevel` printed it.
    root: String,
    /// Whether, and how, its edits are gated.
    gate: Gate,
}

/// What gates a worktree's edits.
enum Gate {
    /// No ledger, or no mode for the worktree: every edit is allowed.
    Open,
    /// A mode: its ledger, and what it was entered with.
    Mode {
        store: Store,
        entered: Entered,
        /// Whether its preconditions were found to hold (they are the same for every file).
        held: bool,
    },
}

impl<L> Worktrees<L>
where
    L: Fn(&str) -> Option<OsString>,
{
    /// No worktree met yet; `lookup` reads the environment, `budget` bounds every wait.
    pub(super) const fn new(lookup: L, budget: Budget) -> Self {
        Self {
            lookup,
            budget,
            anchors: Vec::new(),
            met: Vec::new(),
        }
    }

    /// Judges an edit of `target`, which the OS (or a tool) writes at `resolved`, in the
    /// worktree git finds from `anchor` (its longest existing ancestor directory): `None` to
    /// allow, else the denial with the ledger it is recorded in.
    pub(super) fn judge(
        &mut self,
        anchor: &Path,
        target: &Path,
        resolved: Result<&Path, &io::Error>,
    ) -> Option<Verdict> {
        let index = match self.place_of(anchor) {
            Place::Worktree(index) => index,
            Place::NoRepository => return None,
            Place::Denied(verdict) => return Some(verdict),
        };
        let Some(Worktree {
            root,
            gate: Gate::Mode { entered, .. },
        }) = self.met.get(index)
        else {
            return None;
        };
        let gated = match relative_in(root, target, resolved) {
            Ok(relative) => is_gated(entered, &relative),
            Err(denial) => return Some(self.deny(index, denial)),
        };
        if !gated {
            return None;
        }
        match self.preconditions(index) {
            Ok(None) => None,
            Ok(Some(denial)) | Err(denial) => Some(self.deny(index, denial)),
        }
    }

    /// The ledger seen from `dir` when it exists, opened within the budget, to record a denial
    /// found before any worktree was read.
    pub(super) fn ledger_at(&self, dir: &Path) -> Ledger {
        open_within(dir, &self.lookup, self.budget)
    }

    /// Where `anchor` is, finding and reading its worktree the first time.
    fn place_of(&mut self, anchor: &Path) -> Place {
        if let Some((_, index)) = self.anchors.iter().find(|(known, _)| known == anchor) {
            return index.map_or(Place::NoRepository, Place::Worktree);
        }
        let Ok(timeout) = self.budget.next() else {
            return Place::Denied(Verdict::deny(no_time_left(), Err(RecordError::OutOfTime)));
        };
        let place = match paths::worktree_root_within(anchor, timeout) {
            Ok(root) => self.open(anchor, root),
            Err(PathError::NotARepository { .. }) => Place::NoRepository,
            Err(error) => {
                let denial = unjudged("the worktree cannot be found", &error);
                return Place::Denied(Verdict::deny(denial, self.ledger_at(anchor)));
            }
        };
        match place {
            Place::Worktree(index) => self.anchors.push((anchor.to_path_buf(), Some(index))),
            Place::NoRepository => self.anchors.push((anchor.to_path_buf(), None)),
            Place::Denied(_) => {}
        }
        place
    }

    /// The worktree whose root git printed as `root`, reading its ledger (seen from `anchor`)
    /// and mode when it is new.
    fn open(&mut self, anchor: &Path, root: String) -> Place {
        if let Some(index) = self.met.iter().position(|known| known.root == root) {
            return Place::Worktree(index);
        }
        let (Ok(git), busy) = (self.budget.next(), self.budget.cap(BUSY_TIMEOUT)) else {
            return Place::Denied(Verdict::deny(no_time_left(), Err(RecordError::OutOfTime)));
        };
        let gate = match existing_ledger::open_existing(anchor, &self.lookup, git, busy) {
            // No ledger: no mode was ever entered in the edited file's repository.
            Ok(None) => Gate::Open,
            Ok(Some(store)) => match mode::active(&store, &root) {
                Ok(Some(entered)) => Gate::Mode {
                    store,
                    entered,
                    held: false,
                },
                Ok(None) => Gate::Open,
                Err(error) => {
                    let denial = unjudged("the ledger cannot be read", &error);
                    return Place::Denied(Verdict::deny(denial, Ok(Some(store))));
                }
            },
            Err(error) => {
                let denial = unjudged("the ledger cannot be read", &error);
                return Place::Denied(Verdict::deny(denial, Err(RecordError::Ledger(error))));
            }
        };
        self.met.push(Worktree { root, gate });
        Place::Worktree(self.met.len().saturating_sub(1))
    }

    /// The denial when the mode's preconditions of worktree `index` do not hold, read once:
    /// `Ok(None)` when they hold; `Err` when no time is left to read them.
    fn preconditions(&mut self, index: usize) -> Result<Option<Denial>, Denial> {
        let budget = self.budget;
        let Some(Worktree {
            root,
            gate:
                Gate::Mode {
                    store,
                    entered,
                    held,
                },
        }) = self.met.get_mut(index)
        else {
            return Ok(None);
        };
        if *held {
            return Ok(None);
        }
        if budget.next().is_err() {
            return Err(no_time_left());
        }
        let denial = mode_denial(store, root, entered);
        *held = denial.is_none();
        Ok(denial)
    }

    /// The verdict denying with `denial`, recorded in worktree `index`'s ledger. It ends the
    /// call (the first denial is the answer), so the cache gives its ledger up.
    fn deny(&mut self, index: usize, denial: Denial) -> Verdict {
        let ledger = match mem::take(&mut self.met).into_iter().nth(index) {
            Some(Worktree {
                gate: Gate::Mode { store, .. },
                ..
            }) => Ok(Some(store)),
            Some(Worktree {
                gate: Gate::Open, ..
            })
            | None => Ok(None),
        };
        Verdict::deny(denial, ledger)
    }

    /// How many directories git was asked from.
    #[cfg(test)]
    pub(super) fn anchors_asked(&self) -> usize {
        self.anchors.len()
    }

    /// How many worktrees' ledgers were looked for.
    #[cfg(test)]
    pub(super) fn worktrees_opened(&self) -> usize {
        self.met.len()
    }
}

/// Whether an edit of `relative` (relative to the worktree root) is gated in the mode
/// `entered`: build mode's own rule, else any source file.
fn is_gated(entered: &Entered, relative: &str) -> bool {
    match &entered.mode {
        Active::Build(build) => build::is_gated(build, relative),
        Active::IncompleteBuild | Active::Diagnose => {
            scope::classify(relative, &[]) == PathKind::Source
        }
    }
}

/// The ledger seen from `dir` when it exists, with git and the lock wait within `budget`.
pub(super) fn open_within(
    dir: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    budget: Budget,
) -> Ledger {
    let (Ok(git), busy) = (budget.next(), budget.cap(BUSY_TIMEOUT)) else {
        return Err(RecordError::OutOfTime);
    };
    existing_ledger::open_existing(dir, lookup, git, busy).map_err(RecordError::Ledger)
}

/// The denial for an edit the hook's budget left no time to judge.
fn no_time_left() -> Denial {
    Denial::new(
        Check::Unjudged,
        format!(
            "preconditions: no time is left to judge the edit within the hook's \
             {JUDGE_BUDGET:?} budget, so the edit cannot be checked; try again, with fewer files \
             in one call"
        ),
    )
}
