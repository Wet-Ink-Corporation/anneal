//! `find`'s words before its last action, as far as the skeleton trusts them. The denylist's
//! walk takes every `-exec`, `-execdir`, `-ok` and `-okdir` word as an action, up to the next
//! `;` or `+` (or action word), whatever stands before it; find reads its expression first,
//! so `find . -name -exec x \;` holds no action (`-exec` is `-name`'s pattern). An action
//! counts only when every word of `find` before it reads with certainty by the tables here:
//! leading options, paths, then exact tests, operators and actions, each value of its slot's
//! shape, and each action ending in `;`, or in `+` right after `{}`.

use std::ops::Range;

use super::shape::Shape;
use crate::hook::guard::shell::{Shape as WordShape, Word};

/// Options before the paths that take no value; `-D` takes one, `-O` its level attached.
const LEADING: &str = "-H -L -P";

/// Words of the expression that take no value: operators, tests and actions.
const FLAGS: &str = "! ( ) , -not -a -and -o -or -empty -false -true -nouser -nogroup \
                     -readable -writable -executable -depth -xdev -mount -noleaf -follow \
                     -daystart -ignore_readdir_race -noignore_readdir_race -warn -nowarn \
                     -delete -print -print0 -ls -prune -quit";

/// Words of the expression that take the next word as their value, and its syntax.
const VALUES: [(&str, Shape); 41] = [
    ("-name", Shape::Any),
    ("-iname", Shape::Any),
    ("-path", Shape::Any),
    ("-ipath", Shape::Any),
    ("-wholename", Shape::Any),
    ("-iwholename", Shape::Any),
    ("-lname", Shape::Any),
    ("-ilname", Shape::Any),
    ("-regex", Shape::Any),
    ("-iregex", Shape::Any),
    ("-regextype", Shape::Any),
    ("-newer", Shape::Any),
    ("-anewer", Shape::Any),
    ("-cnewer", Shape::Any),
    ("-samefile", Shape::Any),
    ("-user", Shape::Any),
    ("-group", Shape::Any),
    ("-fstype", Shape::Any),
    ("-context", Shape::Any),
    ("-fprint", Shape::Any),
    ("-fprint0", Shape::Any),
    ("-fls", Shape::Any),
    ("-printf", Shape::Any),
    ("-type", TYPES),
    ("-xtype", TYPES),
    ("-maxdepth", Shape::Digits),
    ("-mindepth", Shape::Digits),
    ("-mtime", Shape::Integer),
    ("-atime", Shape::Integer),
    ("-ctime", Shape::Integer),
    ("-mmin", Shape::Integer),
    ("-amin", Shape::Integer),
    ("-cmin", Shape::Integer),
    ("-used", Shape::Integer),
    ("-links", Shape::Integer),
    ("-inum", Shape::Integer),
    ("-uid", Shape::Integer),
    ("-gid", Shape::Integer),
    ("-size", Shape::Blocks),
    ("-perm", Shape::Mode),
    ("-files0-from", Shape::Any),
];

/// `-type`'s letters.
const TYPES: Shape = Shape::Words("b c d p f l s D");

/// The words that run an action, up to its `;` or `+`.
const ACTIONS: &str = "-exec -execdir -ok -okdir";

/// Whether the space-separated `list` holds `text`.
fn holds(list: &str, text: &str) -> bool {
    list.split_ascii_whitespace().any(|item| item == text)
}

/// The syntax of the value the expression word `text` takes, or `None` for one that takes
/// none.
fn value_of(text: &str) -> Option<Shape> {
    VALUES
        .iter()
        .find(|(option, _)| *option == text)
        .map(|&(_, shape)| shape)
}

/// Where `find`'s words stand: its leading options, its paths, or its expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Leading,
    Paths,
    Expression,
}

/// The index of the first word of `words` from `start` (the word after `find`) up to the end
/// of its last action that does not read with certainty, or `None` when each does. `actions`
/// are the walk's, in order.
pub(super) fn first_uncertain(
    words: &[Word],
    start: usize,
    actions: &[Range<usize>],
) -> Option<usize> {
    let mut actions = actions.iter();
    let mut part = Part::Leading;
    let mut at = start;
    while let Some(word) = words.get(at) {
        let text = word.text.as_str();
        if !word.is_literal() {
            return Some(at);
        }
        let next = at.saturating_add(1);
        if holds(ACTIONS, text) {
            // An action the walk did not read from here (`-exec ;`) is no certain one.
            let Some(range) = actions.next().filter(|range| range.start == next) else {
                return Some(at);
            };
            if !ends_action(words, text, range) {
                return Some(range.end);
            }
            if actions.as_slice().is_empty() {
                return None;
            }
            at = range.end.saturating_add(1);
            part = Part::Expression;
            continue;
        }
        if part == Part::Leading {
            if holds(LEADING, text) || leading_level(text) {
                at = next;
                continue;
            }
            if text == "-D" {
                // Debug options, any word.
                if !words.get(next).is_some_and(|value| Shape::Any.fills(value)) {
                    return Some(next);
                }
                at = next.saturating_add(1);
                continue;
            }
            part = Part::Paths;
        }
        if part == Part::Paths {
            if !(text.starts_with('-') || holds("! ( ) ,", text)) {
                at = next;
                continue;
            }
            part = Part::Expression;
        }
        if holds(FLAGS, text) {
            at = next;
            continue;
        }
        let Some(shape) = value_of(text) else {
            return Some(at);
        };
        if !words.get(next).is_some_and(|value| shape.fills(value)) {
            return Some(next);
        }
        at = next.saturating_add(1);
    }
    // The walk's actions were not all reached in order.
    Some(at)
}

/// Whether `-O` and an optimisation level (`-O3`) is the word `text`.
fn leading_level(text: &str) -> bool {
    text.strip_prefix("-O")
        .is_some_and(|level| level.len() == 1 && level.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Whether the action `keyword` whose command is `range` ends where the walk ended it: at a
/// literal `;`, or for `-exec` and `-execdir` at a `+` right after a literal `{}` (anywhere
/// else a `+` is an argument, and `-ok` takes none).
fn ends_action(words: &[Word], keyword: &str, range: &Range<usize>) -> bool {
    let Some(end) = words.get(range.end).filter(|word| word.is_literal()) else {
        return false;
    };
    match end.text.as_str() {
        ";" => true,
        "+" => {
            !keyword.starts_with("-ok")
                && range
                    .end
                    .checked_sub(1)
                    .filter(|&last| last > range.start)
                    .and_then(|last| words.get(last))
                    // An unquoted `{}` is a brace pattern bash leaves as it is.
                    .is_some_and(|last| last.shape != WordShape::Dynamic && last.text == "{}")
        }
        _ => false,
    }
}
