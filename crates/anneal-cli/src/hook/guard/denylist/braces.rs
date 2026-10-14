//! Brace expansion (`{a,b}`), which the shell applies before anything else, so `rm {-rf,/}`
//! is `rm -rf /`, and `rm "$b"{-rf,/}` is `rm "$b"-rf "$b"/`.
//!
//! A word is expanded when it holds an unquoted pattern character ([`Word::globs`]), whatever
//! its shape. As in bash, a `{`, `,` or `}` that was quoted or escaped ([`Word::quotes`]) is no
//! part of a group, so `bash {-c,'echo ,; rm -rf /'}` is `bash -c 'echo ,; rm -rf /'`; nor are
//! the braces of `${x}`, the text of `$( )` or a variable's known value. A product with no text
//! that nothing quoted went into is no word (`bash {,}` is `bash`), while `''{,}` is two empty
//! words. Sequences (`{1..3}`) are not expanded.

use std::borrow::Cow;
use std::iter::Peekable;
use std::ops::Range;

use crate::hook::guard::shell::{Expansion, Shape, Word};
use crate::hook::guard::{Denial, Rule};

/// The most words brace expansion may make of one command's arguments (H-13).
const MAX_WORDS: usize = 256;

/// The most groups one word may expand: each has at least two alternatives, so more than this
/// makes more than [`MAX_WORDS`] words.
const MAX_GROUPS: usize = 8;

/// More words than [`MAX_WORDS`].
#[derive(Debug)]
struct TooMany;

/// A word's text and where its expansions and quoted parts are in it, in order, borrowed.
#[derive(Debug, Clone, Copy)]
struct Source<'a> {
    text: &'a str,
    expansions: &'a [Expansion],
    quotes: &'a [Range<usize>],
}

impl<'a> From<&'a Word> for Source<'a> {
    fn from(word: &'a Word) -> Self {
        Self {
            text: &word.text,
            expansions: &word.expansions,
            quotes: &word.quotes,
        }
    }
}

/// A word's text and where its expansions and quoted parts are in it, as brace expansion
/// rebuilds it.
#[derive(Debug)]
struct Piece {
    text: String,
    expansions: Vec<Expansion>,
    quotes: Vec<Range<usize>>,
}

impl Piece {
    /// The piece as a source to expand further.
    fn source(&self) -> Source<'_> {
        Source {
            text: &self.text,
            expansions: &self.expansions,
            quotes: &self.quotes,
        }
    }

    /// Whether the piece is a word: it has text, or something quoted or expanded went into it.
    fn is_word(&self) -> bool {
        !self.text.is_empty() || !self.expansions.is_empty() || !self.quotes.is_empty()
    }
}

impl Source<'_> {
    /// The source as an owned piece.
    fn to_piece(self) -> Piece {
        Piece {
            text: self.text.to_owned(),
            expansions: self.expansions.to_vec(),
            quotes: self.quotes.to_vec(),
        }
    }
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
    word.has_pattern() && group(Source::from(word)).is_some()
}

/// Whether `word`'s text is what the shell passes on: a literal word, or one brace expansion
/// made that holds no other pattern character (`-c` and `rm -rf /` from
/// `{bash,-c,'rm -rf /'}`): no expansion, glob, brace or leading `~`, and not read inside
/// `$[ ]`. Only brace expansion makes such a pattern, since the lexer keeps every pattern
/// character it reads in the text, so a word as the lexer read it is this only when literal.
/// A quoted pattern character counts too (`'*'` from `{x,'*'}`), so such a word stays a
/// pattern, as an unquoted variable value with one does.
/// Programs, wrappers and shells read their names and options through this; the entries still
/// see such a word as a pattern, so `rm -rf /tmp/{a,b}` stays denied.
pub(super) fn reads_as_written(word: &Word) -> bool {
    word.is_literal()
        || (word.shape == Shape::Pattern
            && word.expansions.is_empty()
            && !word.arithmetic
            && !word.text.starts_with('~')
            && !word.text.contains(['*', '?', '[', '{']))
}

/// `args` with their brace groups expanded ([`product`] says what each new word is), a product
/// with no text that nothing quoted or expanded went into dropped, as bash drops it.
///
/// # Errors
/// An unparseable-command [`Denial`] when the expansion makes more than [`MAX_WORDS`] words
/// (the words it makes; the others are not counted, as before the program was expanded too).
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
    let mut made: usize = 0;
    for arg in args {
        if !has_group(arg) {
            // The new argument list owns its words: the unexpanded ones are copied beside the
            // expanded ones (only when a brace group is present).
            expanded.push(arg.clone());
            continue;
        }
        let mut pieces = Vec::new();
        expand_text(Source::from(arg), &mut pieces, 0).map_err(too_many)?;
        let before = expanded.len();
        expanded.extend(
            pieces
                .into_iter()
                .filter(Piece::is_word)
                .map(|piece| product(arg, piece)),
        );
        made = made.saturating_add(expanded.len().saturating_sub(before));
        if made > MAX_WORDS {
            return Err(too_many(TooMany));
        }
    }
    Ok(Cow::Owned(expanded))
}

/// The word `piece` is, as a product of `arg`, read from what is left in it rather than from
/// `arg` (#127 review round 4): its own expansions and quoted parts; a run-time shape only
/// while an expansion is left in it, and otherwise a pattern, which programs, wrappers and
/// shells read as written ([`reads_as_written`]) and the entries as a pattern; and quoted only
/// when a quoted part is left. Two fields stay the source word's: where it was read
/// (`arithmetic`), and its unquoted pattern character ([`Word::globs`]), which a product with
/// an expansion left needs for its reading with the expansion empty (`"$b"-f` from
/// `"$b"{-f,x}` is `-f` when `$b` is empty; [`super::readings`]); a product with none left
/// has no such reading, and no group, so the flag changes nothing there.
fn product(arg: &Word, piece: Piece) -> Word {
    let shape = if piece.expansions.is_empty() {
        arg.shape.min(Shape::Pattern)
    } else {
        arg.shape
    };
    Word {
        text: piece.text,
        shape,
        quoted: arg.quoted && !piece.quotes.is_empty(),
        globs: arg.globs,
        expansions: piece.expansions,
        quotes: piece.quotes,
        arithmetic: arg.arithmetic,
    }
}

/// Pushes every word `source` expands to onto `out`; `groups` were expanded on the way here.
fn expand_text(source: Source<'_>, out: &mut Vec<Piece>, groups: usize) -> Result<(), TooMany> {
    let Some(group) = group(source) else {
        if out.len() >= MAX_WORDS {
            return Err(TooMany);
        }
        out.push(source.to_piece());
        return Ok(());
    };
    if groups >= MAX_GROUPS {
        return Err(TooMany);
    }
    for alternative in &group.alternatives {
        match choose(source, &group, alternative) {
            Some(piece) => expand_text(piece.source(), out, groups.saturating_add(1))?,
            // Not reached: the group's ranges are within `text`. Keeping the word unexpanded
            // never drops it.
            None => out.push(source.to_piece()),
        }
    }
    Ok(())
}

/// `source` with `group` replaced by its alternative `chosen`, keeping the expansions and
/// quoted parts before the group, in `chosen` and after the group (each moved to its new
/// place).
fn choose(source: Source<'_>, group: &Group, chosen: &Range<usize>) -> Option<Piece> {
    let text = source.text;
    let prefix = text.get(..group.open)?;
    let alternative = text.get(chosen.start..chosen.end)?;
    let after = group.close.checked_add(1)?;
    let suffix = text.get(after..)?;
    let mut piece = Piece {
        text: [prefix, alternative, suffix].concat(),
        expansions: Vec::with_capacity(source.expansions.len()),
        quotes: Vec::with_capacity(source.quotes.len()),
    };
    let places = Places {
        group,
        chosen,
        after,
        suffix_at: group.open.checked_add(alternative.len())?,
    };
    for expansion in source.expansions {
        if let Placed::At(range) = places.place(&expansion.range)? {
            piece.expansions.push(Expansion {
                range,
                quoted: expansion.quoted,
            });
        }
    }
    for quote in source.quotes {
        if let Placed::At(range) = places.place(quote)? {
            piece.quotes.push(range);
        }
    }
    Some(piece)
}

/// Where the parts of a text go when a group is replaced by its alternative `chosen`.
#[derive(Debug)]
struct Places<'g> {
    group: &'g Group,
    chosen: &'g Range<usize>,
    /// Just past the group's `}`.
    after: usize,
    /// Where the text after the group starts in the new text.
    suffix_at: usize,
}

/// Where a range of a text is once a group is replaced by one of its alternatives.
#[derive(Debug)]
enum Placed {
    /// At this range of the new text.
    At(Range<usize>),
    /// In another alternative: not in the new text.
    Elsewhere,
}

impl Places<'_> {
    /// Where `range` is in the new text, or `None` when a moved offset does not fit, which the
    /// ranges of a text never make. A range never holds a `{`, `,` or `}` of the group, so it
    /// is before the group, in one alternative or after the group; an empty one at an edge
    /// goes with the part it ends or starts (`''` before `{`, in `{'',x}` or after `}`).
    fn place(&self, range: &Range<usize>) -> Option<Placed> {
        Some(if range.end <= self.group.open {
            Placed::At(range.start..range.end)
        } else if range.start >= self.chosen.start && range.end <= self.chosen.end {
            Placed::At(shift(range, self.chosen.start, self.group.open)?)
        } else if range.start >= self.after {
            Placed::At(shift(range, self.after, self.suffix_at)?)
        } else {
            Placed::Elsewhere
        })
    }
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

/// Whether the next of `ahead`'s ranges (in order, none overlapping) covers `at`. Offsets come
/// in ascending order, so a range that ends at or before `at` is dropped for good.
fn covers<'r>(ahead: &mut Peekable<impl Iterator<Item = &'r Range<usize>>>, at: usize) -> bool {
    while ahead.next_if(|range| range.end <= at).is_some() {}
    ahead.peek().is_some_and(|range| range.start <= at)
}

/// The first brace group in `source` (by its `{`) that closes and has a comma at its top
/// level, reading no character inside an expansion or a quoted part (each in order, as the
/// lexer and [`choose`] keep them).
///
/// One pass with a stack of the open braces, so a word of many `{` stays linear (H-13).
fn group(source: Source<'_>) -> Option<Group> {
    let mut open: Vec<Open> = Vec::new();
    let mut first: Option<(Open, usize)> = None;
    let mut expansions = source
        .expansions
        .iter()
        .map(|expansion| &expansion.range)
        .peekable();
    let mut quotes = source.quotes.iter().peekable();
    for (at, c) in source.text.char_indices() {
        if covers(&mut expansions, at) || covers(&mut quotes, at) {
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
