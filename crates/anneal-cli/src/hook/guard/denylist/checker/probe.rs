//! What the merge gate's probe notes as it walks a command: each `gh pr merge`, a
//! [`GH_TARGET_VARIABLES`] the command sets, and how it changed directory before gh.
//!
//! These notes are defence in depth: the gate's invariant rests on `--repo`, which gh reads
//! before `GH_REPO`, `GH_HOST` and the working directory.

use super::{Checker, Entries, Scope, unreadable};
use crate::hook::guard::Denial;
use crate::hook::guard::denylist::braces;
use crate::hook::guard::denylist::gh::{self, PrMerge};
use crate::hook::guard::denylist::wrappers;
use crate::hook::guard::shell::{Shape, Word};

/// The variables that point gh at another repository or host; the probe notes a command that
/// sets or names one (`GH_REPO=o/x gh ...`, `env`, `export`), wherever it does.
const GH_TARGET_VARIABLES: [&str; 2] = ["GH_REPO", "GH_HOST"];

/// The prefix every gh variable name has.
const GH_PREFIX: &str = "GH_";

/// Why a merge in a command that defines a function or an alias cannot be trusted.
const DEFINES: &str = "runs in a command that defines a function or an alias, which may pass \
                       gh other arguments";

/// The most `gh pr merge` calls one command may make before the probe refuses it (H-13).
const MAX_MERGES: usize = 8;

/// The one of [`GH_TARGET_VARIABLES`] `text` sets or names: `NAME`, `NAME=...` or `NAME+=...`.
fn gh_target(text: &str) -> Option<&'static str> {
    GH_TARGET_VARIABLES.into_iter().find(|name| {
        text.strip_prefix(name).is_some_and(|after| {
            after.is_empty() || after.starts_with('=') || after.starts_with("+=")
        })
    })
}

/// Whether `word` holds a brace sequence (`{a..b}`, which brace expansion here does not
/// expand) that may build a gh variable's name: the text before its first `{` starts with
/// `GH_`, or begins `GH_` (`G{H..H}_REPO`), or is empty in an assignment (`{G..G}H_REPO=`).
fn sequence_may_name_gh_variable(word: &Word) -> bool {
    if word.shape != Shape::Pattern {
        return false;
    }
    let text = word.text.as_str();
    let Some((prefix, group)) = text.split_once('{') else {
        return false;
    };
    let sequence = group
        .split_once("..")
        .is_some_and(|(_, after)| after.contains('}'));
    let names = if prefix.is_empty() {
        text.contains('=')
    } else {
        prefix.starts_with(GH_PREFIX) || GH_PREFIX.starts_with(prefix)
    };
    sequence && names
}

impl Checker<'_> {
    /// The `gh pr merge` calls found, in order, each once, each noting a [`GH_TARGET_VARIABLES`]
    /// the command sets anywhere and a `cd`, `pushd` or `popd` it runs anywhere: a function or
    /// trap body is read where it is defined but may run after a later change. Each is unsure
    /// when the command defines a function or an alias anywhere: gh may then run with
    /// arguments other than the ones read (`f(){ gh pr merge "$@"; }; f 5 --repo o/x`), or the
    /// name `gh` may run something else.
    pub(in crate::hook::guard::denylist) fn into_merges(self) -> Vec<PrMerge> {
        let (env, moved) = (self.gh_env, self.moved);
        let defined = self.defines.then_some(DEFINES);
        self.merges
            .into_iter()
            .map(|merge| PrMerge {
                env: env.map(str::to_owned),
                moved: merge.moved.or(moved),
                unsure: merge.unsure.or(defined),
                ..merge
            })
            .collect()
    }

    /// Notes the first word of `words` that sets or names one of [`GH_TARGET_VARIABLES`]
    /// (`GH_REPO`, `GH_REPO=...`, `GH_REPO+=...`), in the probe only. A word holding a brace
    /// group is read as each word it expands to (`export GH_RE{PO,X}=o/x` sets `GH_REPO`).
    ///
    /// # Errors
    /// The unreadable denial when one word expands to more than the brace cap, or holds a
    /// brace sequence that may build a gh variable's name.
    pub(super) fn note_gh_env(&mut self, words: &[Word]) -> Option<Denial> {
        if self.entries != Entries::PrMerge || self.gh_env.is_some() {
            return None;
        }
        for word in words {
            if sequence_may_name_gh_variable(word) {
                return Some(unreadable(format!(
                    "{} may build a gh variable's name by a brace sequence; write the name out",
                    word.text
                )));
            }
            let expanded = match braces::expand(std::slice::from_ref(word)) {
                Ok(expanded) => expanded,
                Err(denial) => return Some(denial),
            };
            self.gh_env = expanded.iter().find_map(|word| gh_target(&word.text));
            if self.gh_env.is_some() {
                break;
            }
        }
        None
    }

    /// Records the `gh pr merge` that `gh <args>` runs in `scope`, if any, with how the command
    /// changed directory before it; past [`MAX_MERGES`] denies.
    pub(super) fn gh(&mut self, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        let merge = PrMerge {
            moved: scope.moved.or(self.moved),
            ..gh::pr_merge(args)?
        };
        if !self.merges.contains(&merge) {
            self.merges.push(merge);
        }
        (self.merges.len() > MAX_MERGES).then(|| {
            unreadable(format!(
                "the command runs more than {MAX_MERGES} gh pr merge calls; merge one pull \
                 request at a time"
            ))
        })
    }

    /// `scope`, noting in the probe how a wrapper of `words` changes the working directory
    /// before its command runs ([`wrappers::chdir`]); an outer one is kept.
    pub(super) fn moved_by_wrapper<'s>(&self, words: &[Word], scope: Scope<'s>) -> Scope<'s> {
        if self.entries != Entries::PrMerge || scope.moved.is_some() {
            return scope;
        }
        Scope {
            moved: wrappers::chdir(words),
            ..scope
        }
    }
}
