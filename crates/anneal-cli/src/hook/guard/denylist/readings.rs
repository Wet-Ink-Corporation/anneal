//! The readings of a word the denylist judges: the word as written and, when it holds both an
//! expansion and an unquoted pattern character, the word with those expansions empty
//! (`"$b"{-rf,/}` is `{-rf,/}` and `"$d".clau*` is `.clau*` when the variable is empty). A word
//! is denied when either reading is, so it is judged at least as strictly as its pattern alone.
//!
//! Every place that judges a word reads it through here: a command's arguments
//! ([`arguments`]) and a redirection's target ([`of`]).

use std::borrow::Cow;

use crate::hook::guard::shell::{Globs, Shape, Word};

/// `word`'s readings: as written, then with its expansions empty when it has that reading.
pub(super) fn of(word: &Word) -> impl Iterator<Item = Cow<'_, Word>> {
    std::iter::once(Cow::Borrowed(word)).chain(emptied(word).map(Cow::Owned))
}

/// `args` with every word that has an emptied reading replaced by it, or `None` when no word
/// has one, so the command needs no second reading.
pub(super) fn arguments(args: &[Word]) -> Option<Vec<Word>> {
    if !args.iter().any(has_emptied) {
        return None;
    }
    let emptied = args
        .iter()
        // The new reading owns its words: the others are copied beside the emptied ones.
        .map(|arg| emptied(arg).unwrap_or_else(|| arg.clone()))
        .collect();
    Some(emptied)
}

/// Whether `word` holds both an expansion and an unquoted pattern character.
fn has_emptied(word: &Word) -> bool {
    word.has_pattern() && !word.expansions.is_empty()
}

/// `word` with every expansion empty (one of its values; an empty value leaves the pattern
/// alone), as a pattern, when it holds both an expansion and an unquoted pattern character. A
/// word whose ranges do not fit its text (which the lexer never makes) has no such reading.
fn emptied(word: &Word) -> Option<Word> {
    if !has_emptied(word) {
        return None;
    }
    let parts = word.outside_expansions()?;
    Some(Word {
        text: parts.concat(),
        shape: Shape::Pattern,
        quoted: word.quoted,
        globs: Globs::Unquoted,
        expansions: Vec::new(),
        arithmetic: word.arithmetic,
    })
}
