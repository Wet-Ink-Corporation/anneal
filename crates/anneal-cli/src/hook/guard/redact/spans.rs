//! Where the quotes, substitutions and words of the source end, read with the lexer's quoting
//! rules: the positions the redaction scanner rewrites between.

use super::Quoting;
use crate::hook::guard::shell::MAX_NESTING;

/// The char at byte `pos` of `text`, if any.
pub(super) fn char_at(text: &str, pos: usize) -> Option<char> {
    text.get(pos..).and_then(|rest| rest.chars().next())
}

/// The byte just past `c` at `pos`.
pub(super) fn after(pos: usize, c: char) -> usize {
    pos.saturating_add(c.len_utf8())
}

/// Characters that end a word outside quotes.
const fn is_meta(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>'
    )
}

/// Where the word starting at `start` ends: the first blank or metacharacter outside any span.
/// `NAME=(` opens an array value. `None` when a span is never closed.
pub(super) fn word_end(text: &str, start: usize, quoting: Quoting, depth: usize) -> Option<usize> {
    let mut pos = start;
    while let Some(c) = char_at(text, pos) {
        if is_meta(c) {
            let array = c == '(' && text.get(start..pos).is_some_and(|word| word.ends_with('='));
            if !array {
                return Some(pos);
            }
            pos = close_group(text, after(pos, c), ')', depth)?;
            continue;
        }
        pos = skip_unit(text, pos, quoting, depth)?;
    }
    Some(pos)
}

/// Whether every quote and substitution `text` opens also closes within it.
pub(super) fn closed(text: &str, quoting: Quoting, depth: usize) -> bool {
    let mut pos = 0;
    while pos < text.len() {
        match skip_unit(text, pos, quoting, depth) {
            Some(next) => pos = next,
            None => return false,
        }
    }
    true
}

/// Past the quoting or substitution starting at `pos` (or the plain character there); `None`
/// when it is never closed.
fn skip_unit(text: &str, pos: usize, quoting: Quoting, depth: usize) -> Option<usize> {
    if depth > MAX_NESTING {
        return None;
    }
    let c = char_at(text, pos)?;
    let next = after(pos, c);
    let deeper = depth.saturating_add(1);
    match (c, char_at(text, next), quoting) {
        ('\\', Some(escaped), _) => Some(after(next, escaped)),
        ('\'', _, Quoting::Bare) => close_single(text, next),
        ('"', _, _) => close_double(text, next, depth),
        ('`', _, _) => close_backtick(text, next),
        ('$', Some('\''), Quoting::Bare) => close_ansi(text, after(next, '\'')),
        ('$', Some('('), _) => close_group(text, after(next, '('), ')', deeper),
        ('$', Some('{'), _) => close_group(text, after(next, '{'), '}', deeper),
        _ => Some(next),
    }
}

/// Past the `'` closing a single-quoted string whose inside starts at `pos`.
pub(super) fn close_single(text: &str, pos: usize) -> Option<usize> {
    let at = text.get(pos..)?.find('\'')?;
    Some(pos.saturating_add(at).saturating_add(1))
}

/// Past the `'` closing an ANSI-C string (`$'…'`, backslash escapes) whose inside starts at
/// `pos`.
pub(super) fn close_ansi(text: &str, mut pos: usize) -> Option<usize> {
    loop {
        match char_at(text, pos)? {
            '\'' => return Some(after(pos, '\'')),
            '\\' => pos = skip_escape(text, pos),
            c => pos = after(pos, c),
        }
    }
}

/// Past the backtick closing a substitution whose inside starts at `pos`.
pub(super) fn close_backtick(text: &str, mut pos: usize) -> Option<usize> {
    loop {
        match char_at(text, pos)? {
            '`' => return Some(after(pos, '`')),
            '\\' => pos = skip_escape(text, pos),
            c => pos = after(pos, c),
        }
    }
}

/// Past the `"` closing a double-quoted string whose inside starts at `pos`. Substitutions
/// inside it are skipped whole; `'` and `$'` are plain characters there.
pub(super) fn close_double(text: &str, mut pos: usize, depth: usize) -> Option<usize> {
    loop {
        let c = char_at(text, pos)?;
        pos = match c {
            '"' => return Some(after(pos, c)),
            '\\' => skip_escape(text, pos),
            '$' | '`' => skip_unit(text, pos, Quoting::Double, depth)?,
            _ => after(pos, c),
        };
    }
}

/// Past the `close` ending a `$( )`, `( )` or `${ }` group whose inside (read unquoted)
/// starts at `pos`.
pub(super) fn close_group(text: &str, mut pos: usize, close: char, depth: usize) -> Option<usize> {
    if depth > MAX_NESTING {
        return None;
    }
    loop {
        let c = char_at(text, pos)?;
        pos = if c == close {
            return Some(after(pos, c));
        } else if c == '(' {
            close_group(text, after(pos, c), ')', depth.saturating_add(1))?
        } else {
            skip_unit(text, pos, Quoting::Bare, depth)?
        };
    }
}

/// Past a backslash at `pos` and the character it escapes.
fn skip_escape(text: &str, pos: usize) -> usize {
    let next = after(pos, '\\');
    char_at(text, next).map_or(next, |escaped| after(next, escaped))
}
