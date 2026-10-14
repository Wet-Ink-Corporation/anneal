//! Brace expansion (`{a,b}`), which the shell applies before anything else, so `rm {-rf,/}`
//! is `rm -rf /`, and `rm "$b"{-rf,/}` is `rm "$b"-rf "$b"/`.
//!
//! A word is expanded when it holds an unquoted pattern character ([`Word::globs`]), whatever
//! its shape. Every `{`, `,` and `}` of such a word outside its expansions' source text is taken
//! as unquoted: a quoted one adds words that do not exist, which can only add denials. The
//! braces of `${x}` and the text of `$( )` are never a group. Sequences (`{1..3}`) are not
//! expanded.

use std::borrow::Cow;
use std::ops::Range;

use crate::hook::guard::shell::{Expansion, Word};
use crate::hook::guard::{Denial, Rule};

/// The most words brace expansion may make of one command's arguments (H-13).
const MAX_WORDS: usize = 256;

/// The most groups one word may expand: each has at least two alternatives, so more than this
/// makes more than [`MAX_WORDS`] words.
const MAX_GROUPS: usize = 8;

/// More words than [`MAX_WORDS`].
#[derive(Debug)]
struct TooMany;

/// A word's text and where its expansions are in it, as brace expansion rebuilds it.
#[derive(Debug)]
struct Piece {
    text: String,
    expansions: Vec<Expansion>,
}

/// A brace group in a text: where its `{` and `}` are, and the range of each alternative.
#[derive(Debug)]
struct Group {
    open: usize,
    close: usize,
    alternatives: Vec<Range<usize>>,
}

/// Whether `word` holds a brace group the shell would expand.
pub(super) fn has_group(word: &Word) -> bool {
    word.has_pattern() && group(&word.text, &word.expansions).is_some()
}

/// `args` with their brace groups expanded; each new word keeps its source word's shape and
/// the expansions that are still in its text.
///
/// # Errors
/// An unparseable-command [`Denial`] when the expansion makes more than [`MAX_WORDS`] words.
pub(super) fn expand(args: &[Word]) -> Result<Cow<'_, [Word]>, Denial> {
    if !args.iter().any(has_group) {
        return Ok(Cow::Borrowed(args));
    }
    let too_many = |TooMany| {
        Denial::new(
            Rule::Unparseable,
            format!("brace expansion makes more than {MAX_WORDS} words"),
        )
    };
    let mut expanded = Vec::new();
    for arg in args {
        if !has_group(arg) {
            // The new argument list owns its words: the unexpanded ones are copied beside the
            // expanded ones (only when a brace group is present).
            expanded.push(arg.clone());
            continue;
        }
        let mut pieces = Vec::new();
        expand_text(&arg.text, &arg.expansions, &mut pieces, 0).map_err(too_many)?;
        expanded.extend(pieces.into_iter().map(|piece| Word {
            text: piece.text,
            shape: arg.shape,
            quoted: arg.quoted,
            globs: arg.globs,
            expansions: piece.expansions,
            arithmetic: arg.arithmetic,
        }));
        if expanded.len() > MAX_WORDS {
            return Err(too_many(TooMany));
        }
    }
    Ok(Cow::Owned(expanded))
}

/// Pushes every word `text` (with `expansions`) expands to onto `out`; `groups` were expanded
/// on the way here.
fn expand_text(
    text: &str,
    expansions: &[Expansion],
    out: &mut Vec<Piece>,
    groups: usize,
) -> Result<(), TooMany> {
    let whole = || Piece {
        text: text.to_owned(),
        expansions: expansions.to_vec(),
    };
    let Some(group) = group(text, expansions) else {
        if out.len() >= MAX_WORDS {
            return Err(TooMany);
        }
        out.push(whole());
        return Ok(());
    };
    if groups >= MAX_GROUPS {
        return Err(TooMany);
    }
    for alternative in &group.alternatives {
        match choose(text, expansions, &group, alternative) {
            Some(piece) => {
                expand_text(
                    &piece.text,
                    &piece.expansions,
                    out,
                    groups.saturating_add(1),
                )?;
            }
            // Not reached: the group's ranges are within `text`. Keeping the word unexpanded
            // never drops it.
            None => out.push(whole()),
        }
    }
    Ok(())
}

/// `text` with `group` replaced by its alternative `chosen`, keeping the expansions before the
/// group, in `chosen` and after the group (each moved to its new place).
fn choose(
    text: &str,
    expansions: &[Expansion],
    group: &Group,
    chosen: &Range<usize>,
) -> Option<Piece> {
    let prefix = text.get(..group.open)?;
    let alternative = text.get(chosen.start..chosen.end)?;
    let after = group.close.checked_add(1)?;
    let suffix = text.get(after..)?;
    let mut piece = Piece {
        text: [prefix, alternative, suffix].concat(),
        expansions: Vec::with_capacity(expansions.len()),
    };
    let suffix_at = group.open.checked_add(alternative.len())?;
    for expansion in expansions {
        let range = &expansion.range;
        let moved = if range.end <= group.open {
            range.start..range.end
        } else if range.start >= chosen.start && range.end <= chosen.end {
            shift(range, chosen.start, group.open)?
        } else if range.start >= after {
            shift(range, after, suffix_at)?
        } else {
            // In another alternative: not in this word.
            continue;
        };
        piece.expansions.push(Expansion {
            range: moved,
            quoted: expansion.quoted,
        });
    }
    Some(piece)
}

/// `range`, which starts at or after `from`, moved so that `from` is at `to`.
fn shift(range: &Range<usize>, from: usize, to: usize) -> Option<Range<usize>> {
    let start = range.start.checked_sub(from)?.checked_add(to)?;
    let end = range.end.checked_sub(from)?.checked_add(to)?;
    Some(start..end)
}

/// A `{` not yet closed: where it is, and the commas at its top level.
#[derive(Debug)]
struct Open {
    at: usize,
    commas: Vec<usize>,
}

/// The first brace group in `text` (by its `{`) that closes and has a comma at its top level,
/// reading no character inside `expansions` (in order, as the lexer and [`choose`] keep them).
///
/// One pass with a stack of the open braces, so a word of many `{` stays linear (H-13).
fn group(text: &str, expansions: &[Expansion]) -> Option<Group> {
    let mut open: Vec<Open> = Vec::new();
    let mut first: Option<(Open, usize)> = None;
    let mut ahead = expansions.iter().peekable();
    for (at, c) in text.char_indices() {
        while ahead
            .next_if(|expansion| expansion.range.end <= at)
            .is_some()
        {}
        if ahead
            .peek()
            .is_some_and(|expansion| expansion.range.start <= at)
        {
            continue;
        }
        match c {
            '{' => open.push(Open {
                at,
                commas: Vec::new(),
            }),
            ',' => {
                if let Some(innermost) = open.last_mut() {
                    innermost.commas.push(at);
                }
            }
            '}' => {
                let Some(closed) = open.pop() else {
                    continue;
                };
                let earlier = first.as_ref().is_none_or(|(best, _)| closed.at < best.at);
                if !closed.commas.is_empty() && earlier {
                    first = Some((closed, at));
                }
            }
            _ => {}
        }
    }
    let (open, close) = first?;
    let mut alternatives = Vec::with_capacity(open.commas.len().saturating_add(1));
    let mut start = open.at.checked_add(1)?;
    for comma in open.commas.into_iter().chain([close]) {
        alternatives.push(start..comma);
        start = comma.checked_add(1)?;
    }
    Some(Group {
        open: open.at,
        close,
        alternatives,
    })
}
