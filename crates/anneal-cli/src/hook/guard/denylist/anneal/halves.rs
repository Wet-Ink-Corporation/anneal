//! The two halves of `--actor human` that the words of one call may hold, wherever each stands,
//! for an anneal call the guard cannot read ([`super::Seen`]).

use crate::hook::guard::shell::Word;

use super::{ACTOR, HUMAN, PATTERN, braces, may_be, may_be_several, written_start};

/// The two halves of `--actor human` that words may hold, wherever each stands: the flag, a
/// literal `--actor`; and the value, one word that may be `human` ([`may_be`]: `human`,
/// `"$who"`, `"${H:-human}"`, `"$(echo human)"`), a word that may become several words
/// ([`may_be_several`]: `"$@"`, `$SUB`) excepted, since it stands for words written
/// elsewhere, unless it comes right after a literal `--actor` (`--actor $WHO`). A word whose written start reaches `--actor=` and that may be `--actor=human`
/// (`--actor=human`, `"--actor=${H:-human}"`) is both. The value of an assignment
/// (`a[2]=--actor`, `a[3]=human`) is read as such a word too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::hook::guard::denylist) struct Halves {
    /// A literal `--actor`.
    flag: bool,
    /// A word that may be `human`.
    value: bool,
}

impl Halves {
    /// The halves `words` hold. A word right after a literal `--actor` is its value as clap
    /// reads it, so there it is the value half whenever it may be `human`, even when it may
    /// become several words (`--actor $WHO`, `--actor "${WHO@L}"`).
    pub(super) fn of(words: &[Word]) -> Self {
        let (halves, _) =
            words
                .iter()
                .fold((Self::default(), false), |(halves, after_flag), word| {
                    let assigned = assigned(word);
                    let whole = may_be_human_actor(word);
                    let flag = is_actor_flag(word, assigned.as_ref());
                    let value = (after_flag && may_be(word, HUMAN))
                        || may_be_human(word, assigned.as_ref());
                    let word_halves = Self {
                        flag: whole || flag,
                        value: whole || value,
                    };
                    (halves.union(word_halves), flag)
                });
        halves
    }

    /// The halves either holds.
    pub(super) fn union(self, other: Self) -> Self {
        Self {
            flag: self.flag || other.flag,
            value: self.value || other.value,
        }
    }

    /// Whether both halves are held.
    pub(super) fn both(self) -> bool {
        self.flag && self.value
    }
}

/// Whether `word` is a literal `--actor`, or assigns one (`a[2]=--actor`); `assigned` is
/// [`assigned`] of `word`.
fn is_actor_flag(word: &Word, assigned: Option<&Assigned<'_>>) -> bool {
    (braces::reads_as_written(word) && word.text == ACTOR)
        || assigned.is_some_and(|value| value.whole && value.written == ACTOR)
}

/// Whether `word`, one word, may be `human`, or assigns a value that may be; `assigned` is
/// [`assigned`] of `word`.
fn may_be_human(word: &Word, assigned: Option<&Assigned<'_>>) -> bool {
    (!may_be_several(word) && may_be(word, HUMAN))
        || assigned.is_some_and(|value| value.may_be(HUMAN))
}

/// Whether `word`'s written start reaches `--actor=` and the word may be `--actor=human`.
fn may_be_human_actor(word: &Word) -> bool {
    const WHOLE: &str = "--actor=human";
    written_start(word).starts_with("--actor=") && may_be(word, WHOLE)
}

/// The value an assignment word gives (`NAME=value`, `NAME+=value`, `NAME[sub]=value`), as far
/// as it is written.
struct Assigned<'w> {
    /// The value's text up to its first expansion or pattern character.
    written: &'w str,
    /// Whether the whole value is written: no expansion or pattern character follows.
    whole: bool,
}

impl Assigned<'_> {
    /// Whether the value may be `text` once expanded.
    fn may_be(&self, text: &str) -> bool {
        if self.whole {
            self.written == text
        } else {
            text.starts_with(self.written)
        }
    }
}

/// The value `word` assigns, when it is shaped as an assignment: a name (a letter or `_`, then
/// letters, digits and `_`), an optional `[subscript]`, then `=` or `+=`.
fn assigned(word: &Word) -> Option<Assigned<'_>> {
    let text = word.text.as_str();
    let name_end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|&at| at > 0 && !text.starts_with(|c: char| c.is_ascii_digit()))?;
    let rest = text.get(name_end..)?;
    let rest = match rest.strip_prefix('[') {
        Some(subscript) => subscript.split_once(']')?.1,
        None => rest,
    };
    let value = rest.strip_prefix("+=").or_else(|| rest.strip_prefix('='))?;
    let from = text.len().saturating_sub(value.len());
    let end = word
        .expansions
        .iter()
        .map(|expansion| expansion.range.start)
        .filter(|&start| start >= from)
        .chain(value.find(PATTERN).map(|at| from.saturating_add(at)))
        .min();
    Some(Assigned {
        written: text.get(from..end.unwrap_or(text.len()))?,
        whole: end.is_none(),
    })
}
