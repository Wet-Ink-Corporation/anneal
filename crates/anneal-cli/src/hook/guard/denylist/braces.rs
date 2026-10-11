//! Brace expansion (`{a,b}`), which the shell applies before anything else, so `rm {-rf,/}`
//! is `rm -rf /`.
//!
//! Every `{`, `,` and `}` of a pattern word is taken as unquoted: a quoted one adds words that
//! do not exist, which can only add denials. Sequences (`{1..3}`) are not expanded.

use std::borrow::Cow;

use crate::hook::guard::shell::{Shape, Word};
use crate::hook::guard::{Denial, Rule};

/// The most words brace expansion may make of one command's arguments (H-13).
const MAX_WORDS: usize = 256;

/// The most groups one word may expand: each has at least two alternatives, so more than this
/// makes more than [`MAX_WORDS`] words.
const MAX_GROUPS: usize = 8;

/// More words than [`MAX_WORDS`].
#[derive(Debug)]
struct TooMany;

/// A brace group in a word: the text before it, its alternatives, and the text after it.
#[derive(Debug)]
struct Group<'a> {
    prefix: &'a str,
    alternatives: Vec<&'a str>,
    suffix: &'a str,
}

/// Whether `word` holds a brace group the shell would expand.
pub(super) fn has_group(word: &Word) -> bool {
    word.shape == Shape::Pattern && group(&word.text).is_some()
}

/// `args` with their brace groups expanded; each new word keeps its source word's shape.
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
        let mut texts = Vec::new();
        expand_text(&arg.text, &mut texts, 0).map_err(too_many)?;
        expanded.extend(texts.into_iter().map(|text| Word {
            text,
            shape: arg.shape,
            quoted: arg.quoted,
            expansions: Vec::new(),
        }));
        if expanded.len() > MAX_WORDS {
            return Err(too_many(TooMany));
        }
    }
    Ok(Cow::Owned(expanded))
}

/// Pushes every word `text` expands to onto `out`; `groups` were expanded on the way here.
fn expand_text(text: &str, out: &mut Vec<String>, groups: usize) -> Result<(), TooMany> {
    let Some(group) = group(text) else {
        if out.len() >= MAX_WORDS {
            return Err(TooMany);
        }
        out.push(text.to_owned());
        return Ok(());
    };
    if groups >= MAX_GROUPS {
        return Err(TooMany);
    }
    for alternative in &group.alternatives {
        let text = format!("{}{alternative}{}", group.prefix, group.suffix);
        expand_text(&text, out, groups.saturating_add(1))?;
    }
    Ok(())
}

/// A `{` not yet closed: where it is, and the commas at its top level.
#[derive(Debug)]
struct Open {
    at: usize,
    commas: Vec<usize>,
}

/// The first brace group in `text` (by its `{`) that closes and has a comma at its top level.
///
/// One pass with a stack of the open braces, so a word of many `{` stays linear (H-13).
fn group(text: &str) -> Option<Group<'_>> {
    let mut open: Vec<Open> = Vec::new();
    let mut first: Option<(Open, usize)> = None;
    for (at, c) in text.char_indices() {
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
        alternatives.push(text.get(start..comma)?);
        start = comma.checked_add(1)?;
    }
    Some(Group {
        prefix: text.get(..open.at)?,
        alternatives,
        suffix: text.get(close.checked_add(1)?..)?,
    })
}
