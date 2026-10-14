//! The readings of a word the denylist judges: the word as written and, when it holds both an
//! expansion and an unquoted pattern character, the word with those expansions empty
//! (`"$b"{-rf,/}` is `{-rf,/}` and `"$d".clau*` is `.clau*` when the variable is empty). A word
//! is denied when either reading is, so it is judged at least as strictly as its pattern alone.
//!
//! Every place that judges a word reads it through here: a command's arguments
//! ([`arguments`]) and a redirection's target ([`of`]). Whether a word's values may be an option
//! cluster, whatever its variables hold and its globs match, is [`may_start_option_cluster`]; it
//! judges each word as one word, so word splitting of an unquoted expansion and `"$@"` (which
//! may make a separate `-rf` word) is a known gap.

use std::borrow::Cow;

use crate::hook::guard::shell::{Expansion, Globs, Shape, Word};

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

/// Whether one of `word`'s values may be an option cluster (`-rf`): its first part may be a `-`
/// (an expansion, which may hold one; an unquoted `?`, `*` or bracket expression, which may
/// match a file named `-rf`; or a written `-`), and every later written character is an ASCII
/// letter, digit or `-`, which the expansions and globs may join into a cluster. So `"$b"rf`
/// (`b=-`), `?rf` and `-$x` may be one, and `"$f".txt`, `x"$b"` and `"$d"/x` may not. A bracket
/// expression may match `-` unless it is a plain set that names none (`[.]`): a negated one
/// (`[!a]`) or one with a class (`[[:punct:]]`) may, without naming it. A literal word is exactly
/// its text, which `wrappers::may_be_option` judges, so it is never one here.
///
/// Each expansion is taken as text inside this one word, as a quoted one is. An unquoted
/// expansion, `"$@"` or `"${a[@]}"` may split into several words, one of them `-rf` (`x$b` with
/// `b=' -rf'`, `x"$@"`): that word splitting is not judged, a known gap. So are extended globs
/// (`@(-rf)`, `!(x)`), whose `(` is read as a written character.
pub(super) fn may_start_option_cluster(word: &Word) -> bool {
    if word.is_literal() {
        return false;
    }
    let mut first = true;
    for part in parts(word) {
        let fits = match part {
            Part::Any => true,
            // Past the first part, a set may match a letter or a digit.
            Part::Set { dash } => dash || !first,
            Part::Written(c) => c == '-' || (!first && c.is_ascii_alphanumeric()),
        };
        if !fits {
            return false;
        }
        first = false;
    }
    !first
}

/// A part of a word as the shell expands it, for [`may_start_option_cluster`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// An expansion, `?` or `*`: any text, a `-` included.
    Any,
    /// A bracket expression: one character, and whether it may be `-`.
    Set { dash: bool },
    /// A character as written.
    Written(char),
}

/// `word`'s parts in order. A `?`, `*` or `[` is taken as an unquoted glob whenever the word
/// holds one ([`Word::globs`] does not say which), which can only add denials.
fn parts(word: &Word) -> Parts<'_> {
    Parts {
        text: &word.text,
        expansions: &word.expansions,
        globs: word.has_pattern(),
        at: 0,
    }
}

/// The iterator [`parts`] returns: where it is in the text, and the expansions not yet reached.
#[derive(Debug)]
struct Parts<'a> {
    text: &'a str,
    expansions: &'a [Expansion],
    globs: bool,
    at: usize,
}

impl Iterator for Parts<'_> {
    type Item = Part;

    fn next(&mut self) -> Option<Part> {
        // An expansion that starts before `at` was inside a bracket expression already read.
        while let [expansion, rest @ ..] = self.expansions
            && expansion.range.start < self.at
        {
            self.expansions = rest;
        }
        if let [expansion, rest @ ..] = self.expansions
            && expansion.range.start == self.at
        {
            self.expansions = rest;
            // A range that ends where it starts (which the lexer never makes) still moves on.
            self.at = expansion.range.end.max(self.at.saturating_add(1));
            return Some(Part::Any);
        }
        let c = self.text.get(self.at..)?.chars().next()?;
        let after = self.at.saturating_add(c.len_utf8());
        let glob = match c {
            '?' | '*' if self.globs => Some((after, Part::Any)),
            '[' if self.globs => bracket(self.text, after, self.expansions),
            _ => None,
        };
        let (end, part) = glob.unwrap_or((after, Part::Written(c)));
        self.at = end;
        Some(part)
    }
}

/// The bracket expression whose `[` ends at `from` in `text`: where it closes (just past its
/// `]`) and its [`Part::Set`], or `None` when it does not close, so the `[` is a plain
/// character. A leading `!` or `^` negates it, a `]` right after that is a member, and
/// `[:class:]`, `[=c=]` and `[.c.]` are read whole. An expansion inside (`[$c]`,
/// `[!$(echo ])]`) is jumped over: it may hold any member, `-` and a negating `!` included, and
/// a `]` in its source text never closes the set. Each byte is read once, so a word of many `[`
/// stays linear (H-13).
fn bracket(text: &str, from: usize, expansions: &[Expansion]) -> Option<(usize, Part)> {
    let bytes = text.as_bytes();
    let negated = matches!(bytes.get(from), Some(b'!' | b'^'));
    let mut at = from.saturating_add(usize::from(negated));
    if bytes.get(at) == Some(&b']') {
        at = at.saturating_add(1);
    }
    // A negated set, or one with a class or an expansion, may match `-` without naming it.
    let mut dash = negated;
    let mut ahead = expansions;
    // The kind (`:`, `=` or `.`) of the class being read, if any.
    let mut class = None;
    loop {
        while let [expansion, rest @ ..] = ahead
            && expansion.range.start < at
        {
            ahead = rest;
        }
        if let [expansion, rest @ ..] = ahead
            && expansion.range.start == at
        {
            dash = true;
            ahead = rest;
            // A range that ends where it starts (which the lexer never makes) still moves on.
            at = expansion.range.end.max(at.saturating_add(1));
            continue;
        }
        let byte = *bytes.get(at)?;
        let next = at.saturating_add(1);
        at = match (class, byte, bytes.get(next)) {
            (Some(kind), _, Some(&b']')) if byte == kind => {
                class = None;
                next.saturating_add(1)
            }
            // Inside a class, every other byte (`]` and `-` included) is part of its name.
            (None, b']', _) => return Some((next, Part::Set { dash })),
            (None, b'[', Some(&kind @ (b':' | b'=' | b'.'))) => {
                dash = true;
                class = Some(kind);
                next.saturating_add(1)
            }
            (None, b'-', _) => {
                dash = true;
                next
            }
            _ => next,
        };
    }
}
