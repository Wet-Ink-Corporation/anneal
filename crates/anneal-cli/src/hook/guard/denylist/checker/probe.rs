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
use crate::hook::guard::shell::{Shape, SimpleCommand, Stdin, Word};

/// The variables that point gh at another repository or host; the probe notes a command that
/// sets or names one (`GH_REPO=o/x gh ...`, `env`, `export`), wherever it does.
const GH_TARGET_VARIABLES: [&str; 2] = ["GH_REPO", "GH_HOST"];

/// The prefix every gh variable name has.
const GH_PREFIX: &str = "GH_";

/// The most `gh pr merge` calls one command may make before the probe refuses it (H-13).
const MAX_MERGES: usize = 8;

/// A gh call whose first operands are known only at run time (`gh "$@"`).
pub(super) const RUN_TIME_GH_CALL: &str = "a gh call whose subcommand is known only at run time";

/// A program known only at run time (`${GH:-gh}`, `g$x`).
pub(super) const RUN_TIME_PROGRAM: &str = "a program known only at run time";

/// A function, alias or `hash -p` definition, or a `BASH_FUNC_` export.
pub(super) const DEFINITION: &str = "a function, alias or hash -p definition";

/// A word naming gh where the probe reads no gh call with it (`xargs gh`, `parallel gh`,
/// `ln -s /usr/bin/gh`).
const GH_WORD: &str = "gh where the probe cannot read its arguments";

/// A `gh api` call, which may call the merge endpoint (`gh api -X PUT .../pulls/7/merge`).
const GH_API: &str = "gh api, which may call the merge endpoint";

/// How a gh word or call of a command may hide a merge from the probe, ranked: a later
/// variant outranks an earlier one, and the strongest seen is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum GhHides {
    /// Every word naming gh is a call read in full, and none is `gh api`.
    Nothing,
    /// A simple command has more words naming gh than gh calls read in full ([`GH_WORD`]).
    UnreadWord,
    /// A `gh api` call ([`GH_API`]).
    Api,
}

impl GhHides {
    /// How the command may run a merge the probe cannot read, if it may.
    fn how(self) -> Option<&'static str> {
        match self {
            Self::Nothing => None,
            Self::UnreadWord => Some(GH_WORD),
            Self::Api => Some(GH_API),
        }
    }
}

/// The word whose presence in the command's words, in any case, arms the tripwire.
const MERGE_WORD: &str = "merge";

/// The names gh's program file has, compared without ASCII case.
const GH_NAMES: [&str; 2] = ["gh", "gh.exe"];

/// Whether `text` holds [`MERGE_WORD`] in any case.
fn holds_merge(text: &str) -> bool {
    text.to_lowercase().contains(MERGE_WORD)
}

/// Whether the last path component of `text` (after `/` or `\`) is one of [`GH_NAMES`].
fn names_gh(text: &str) -> bool {
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    GH_NAMES.iter().any(|gh| name.eq_ignore_ascii_case(gh))
}

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
    /// trap body is read where it is defined but may run after a later change.
    pub(in crate::hook::guard::denylist) fn into_merges(self) -> Vec<PrMerge> {
        let (env, moved) = (self.gh_env, self.moved);
        self.merges
            .into_iter()
            .map(|merge| PrMerge {
                env: env.map(str::to_owned),
                moved: merge.moved.or(moved),
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

    /// How the command may run a merge the probe cannot read, if it may: [`RUN_TIME_GH_CALL`]
    /// whenever it makes one; else, when a word holds `merge`, the first of
    /// [`RUN_TIME_PROGRAM`] and [`DEFINITION`], then [`GH_API`] when it runs `gh api`, then
    /// [`GH_WORD`] when a simple command has more words naming gh than the probe read gh calls
    /// in it in full (`gh pr merge 3 --disable-auto` is read; `xargs gh`, `parallel gh` and
    /// `ln -s /usr/bin/gh` are not).
    pub(in crate::hook::guard::denylist) fn unseen(&self) -> Option<&'static str> {
        if self.unseen_gh_call {
            return Some(RUN_TIME_GH_CALL);
        }
        if !self.mentions_merge {
            return None;
        }
        self.unseen.or_else(|| self.gh_hides.how())
    }

    /// Notes, in the probe only, whether `command`'s words (after quote removal, so `'mer'ge`
    /// and `me\rge` read `merge`) or the here-document or here-string it reads hold `merge`.
    /// Substitutions and `bash -c` strings are lexed into commands of their own, so their
    /// words are noted too.
    pub(super) fn note_mentions(&mut self, command: &SimpleCommand) {
        if self.entries != Entries::PrMerge {
            return;
        }
        for word in &command.words {
            self.mentions_merge |= holds_merge(&word.text);
        }
        if let Stdin::Text(body) = &command.stdin {
            self.mentions_merge |= holds_merge(&body.text);
        }
    }

    /// Starts reading one simple command's gh calls; the count of the command it nests in is
    /// returned, for [`Checker::leave_command`] to restore.
    pub(super) fn enter_command(&mut self) -> usize {
        std::mem::take(&mut self.gh_calls_here)
    }

    /// Ends reading a simple command whose words, with its variables put in, are `words`:
    /// notes, in the probe only, a word naming gh that no gh call read in full in it pairs
    /// with (`$G pr view 1` with `G=gh` is a call; `xargs gh` is not), and restores the count
    /// `outer` of the command it nests in.
    pub(super) fn leave_command(&mut self, words: &[Word], outer: usize) {
        if self.entries == Entries::PrMerge {
            let named = words.iter().filter(|word| names_gh(&word.text)).count();
            if named > self.gh_calls_here {
                self.gh_hides = self.gh_hides.max(GhHides::UnreadWord);
            }
        }
        self.gh_calls_here = outer;
    }

    /// Notes `how` the command may run a call the probe cannot read ([`RUN_TIME_PROGRAM`],
    /// [`DEFINITION`]), in the probe only; the first one is kept.
    pub(super) fn note_unseen(&mut self, how: &'static str) {
        if self.entries == Entries::PrMerge {
            self.unseen.get_or_insert(how);
        }
    }

    /// Records the `gh pr merge` that `gh <args>` runs in `scope`, if any, with how the command
    /// changed directory before it; past [`MAX_MERGES`] denies. A call whose first operands are
    /// known only at run time is noted ([`RUN_TIME_GH_CALL`]), as is `gh api` ([`GH_API`]), and
    /// one read in full is counted against the words naming gh in its simple command (see
    /// [`Checker::leave_command`]).
    pub(super) fn gh(&mut self, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        self.unseen_gh_call |= gh::call_unseen(args);
        if gh::calls_api(args) {
            self.gh_hides = GhHides::Api;
        }
        // A bare `gh` reads as no call: `command -v gh` unwraps to it, though it only names gh.
        if !scope.appended && self.argument_depth == 0 && !args.is_empty() {
            self.gh_calls_here = self.gh_calls_here.saturating_add(1);
        }
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
