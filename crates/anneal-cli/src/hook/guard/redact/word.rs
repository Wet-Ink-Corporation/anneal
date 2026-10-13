//! A word the words before it left readable: rewritten in place, then judged as bash reads it.
//!
//! The rewrite ([`Scanner::rewrite_word`]) keeps as much of the raw word as it can: what stays
//! readable before a secret, the inside of each quote scanned as text. Its rules read raw text,
//! whose word boundaries are not bash's: a quote may close inside a header's value, and a line
//! continuation (`\` and a newline) is no character to bash. So the word is also judged whole
//! ([`judge`]) on its [`dequoted`] text, as bash passes it on: quotes removed, line
//! continuations dropped, substitutions kept as written. That judgement decides what the word
//! owes the next word (never anything, for an assignment), and when the rewrite, read back the
//! same way, still holds a secret, the whole raw word is hidden instead.

use super::spans::{Span, SpanKind, Unit, closed, unit_at};
use super::{Pending, QUOTING, Quoting, REDACTED, Scanner, redact_nested};
use crate::decide::redact::judge::{Owes, judge};
use crate::decide::redact::rules::is_assignment;
use crate::decide::redact::tools;
use crate::decide::redact::{
    Scheme, Secret, Then, find_secret, looks_like_token, scheme, split_punctuation, url_userinfo,
};

impl Scanner<'_, '_> {
    /// A word not hidden by the words before it, as the module documentation says.
    pub(super) fn readable_word(&mut self, raw: &str) {
        let tool = self.tool;
        let start = self.out.len();
        self.rewrite_word(raw);
        let text = dequoted(raw, self.quoting, self.depth);
        self.pending = if is_assignment(&text) {
            Pending::Nothing
        } else {
            stronger(self.pending, judge(&text, tool).owes)
        };
        let written = dequoted(
            self.out.get(start..).unwrap_or_default(),
            self.quoting,
            self.depth,
        );
        if judge(&written, tool).secret {
            // `start` was the output's length, so it is a char boundary.
            self.out.truncate(start);
            self.out.push_str(REDACTED);
        }
    }

    /// Rewrites a readable word: its own secret, a token, or the word with any URL userinfo
    /// redacted; then what its raw text says about the next words.
    fn rewrite_word(&mut self, raw: &str) {
        let secret = find_secret(raw, self.tool)
            .filter(|secret| closed(&secret.shown, self.quoting, self.depth));
        if let Some(secret) = secret {
            self.write_secret(&secret);
        } else if let Some((lead, trail)) = plain_token(raw) {
            self.out.push_str(lead);
            self.out.push_str(REDACTED);
            self.out.push_str(trail);
        } else {
            self.pending = self.copy_word(raw);
        }
        if self.pending == Pending::Nothing {
            self.pending = match scheme(raw) {
                Some(Scheme::Bearer) => Pending::Word,
                Some(Scheme::Basic) => Pending::BasicWord,
                None => Pending::Nothing,
            };
        }
        if let Some(tool) = self.tool.as_mut()
            && tool.note(raw)
        {
            self.pending = Pending::Word;
        }
        if let Some(tool) = tools::known(raw) {
            self.tool = Some(tool);
        }
    }

    /// Writes a word's secret: what stays readable, the mark, and what the next words are.
    fn write_secret(&mut self, secret: &Secret<'_>) {
        self.out.push_str(&secret.shown);
        if secret.redacts {
            self.out.push_str(REDACTED);
        }
        match secret.then {
            Then::Nothing => {}
            // An attached `Bearer` (`X-Auth-Token:Bearer xyz`) went with its word; the next
            // word is its credential, as for a credential option's value.
            Then::NextWord | Then::AttachedScheme(Scheme::Bearer) => self.pending = Pending::Word,
            Then::Header => self.pending = Pending::Rest,
            Then::AttachedHeader => self.pending = Pending::AttachedValue,
            Then::AttachedScheme(Scheme::Basic) => self.pending = Pending::BasicWord,
        }
    }

    /// Copies a word with the userinfo of each URL in it (`https://user:token@host`) redacted;
    /// a URL inside a quote is found by the scan of the quote's inside. Returns what a quote
    /// ending the word says about the next word ([`units`]).
    fn copy_word(&mut self, raw: &str) -> Pending {
        let mut rest = raw;
        while let Some((start, end)) = url_userinfo(rest)
            && let (Some(head), Some(tail)) = (rest.get(..start), rest.get(end..))
            && !head.contains(QUOTING)
        {
            self.out.push_str(head);
            self.out.push_str(REDACTED);
            rest = tail;
        }
        units(rest, self.quoting, self.depth, self.out)
    }
}

/// The stronger of what the rewrite and the judgement say the next word is: the one that
/// hides more.
const fn stronger(rewrite: Pending, owes: Owes) -> Pending {
    match (owes, rewrite) {
        (Owes::Header, _) | (_, Pending::Rest) => Pending::Rest,
        (Owes::Word, _) | (_, Pending::Word) => Pending::Word,
        (Owes::BasicWord, _) | (_, Pending::BasicWord) => Pending::BasicWord,
        (Owes::Nothing, other) => other,
    }
}

/// The punctuation around an unquoted word that looks like a token (`ghp_…`), so the token
/// goes whole; a quoted one is found by the scan of the quote's inside.
fn plain_token(raw: &str) -> Option<(&str, &str)> {
    if raw.contains(QUOTING) {
        return None;
    }
    let (lead, core, trail) = split_punctuation(raw);
    looks_like_token(core).then_some((lead, trail))
}

/// The text of the word `raw` (read with `quoting` at nesting `depth`) as bash passes it on:
/// quotes removed, a line continuation dropped, an escape replaced by the character it
/// escapes, and a substitution kept as written, since its value is known only when it runs.
pub(super) fn dequoted(raw: &str, quoting: Quoting, depth: usize) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut pos = 0;
    while pos < raw.len() {
        let unit = unit_at(raw, pos, quoting, depth);
        let end = match unit {
            Unit::Plain(end) => end,
            Unit::Span(Some(span)) => span.end,
            Unit::Span(None) => raw.len(),
        };
        let piece = raw.get(pos..end).unwrap_or_default();
        match unit {
            Unit::Plain(_) => push_plain(piece, quoting, &mut text),
            Unit::Span(Some(span)) => push_span(raw, span, depth, &mut text),
            Unit::Span(None) => text.push_str(piece),
        }
        pos = end;
    }
    text
}

/// A character, or `\` and the character it escapes (inside `"…"` only `$`, a backtick, `"`,
/// `\` and a newline are escaped; a line continuation is dropped).
fn push_plain(piece: &str, quoting: Quoting, text: &mut String) {
    match piece.strip_prefix('\\') {
        Some("\n") => {}
        Some(escaped) if quoting == Quoting::Bare || escaped.starts_with(['$', '`', '"', '\\']) => {
            text.push_str(escaped);
        }
        Some(_) | None => text.push_str(piece),
    }
}

/// The text of a closed span of `raw`.
fn push_span(raw: &str, span: Span, depth: usize, text: &mut String) {
    let inner = raw.get(span.start..span.inner_end()).unwrap_or_default();
    match span.kind {
        SpanKind::Single => text.push_str(inner),
        SpanKind::Double => {
            text.push_str(&dequoted(inner, Quoting::Double, depth.saturating_add(1)));
        }
        SpanKind::Ansi => {
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    text.extend(chars.next());
                } else {
                    text.push(c);
                }
            }
        }
        SpanKind::Substitution => {
            text.push_str(span.open);
            text.push_str(inner);
            text.push_str(span.close);
        }
    }
}

/// Copies a word whose spans are all closed, scanning the inside of each quote or
/// substitution as text.
///
/// A span whose inside ends in a secret still owed (a credential's value, a header's) hides
/// the rest of the word when the word goes on after its close (`"X-Api-Key:"secret`). A quote
/// that ends the word and leaves the next word secret (`"X-Auth-Token:Bearer" xyz`) returns
/// that for the outer scan; a substitution returns nothing, since its inside is a command and
/// the word after it may be one (`x=$(echo Bearer) rm -rf /p`).
fn units(raw: &str, quoting: Quoting, depth: usize, out: &mut String) -> Pending {
    let mut pos = 0;
    while pos < raw.len() {
        let span = match unit_at(raw, pos, quoting, depth) {
            Unit::Plain(end) => {
                out.push_str(raw.get(pos..end).unwrap_or_default());
                pos = end;
                continue;
            }
            // A span `word_end` found closed closes here too; should one not, redact the rest.
            Unit::Span(None) => {
                out.push_str(REDACTED);
                return Pending::Nothing;
            }
            Unit::Span(Some(span)) => span,
        };
        out.push_str(span.open);
        let inner = raw.get(span.start..span.inner_end()).unwrap_or_default();
        let owed = redact_nested(inner, span.quoting(), depth, out);
        out.push_str(span.close);
        pos = span.end;
        match after_span(owed, span.kind, pos < raw.len()) {
            AfterSpan::ReadOn => {}
            AfterSpan::Secret => {
                out.push_str(REDACTED);
                return Pending::Nothing;
            }
            AfterSpan::Next(pending) => return pending,
        }
    }
    Pending::Nothing
}

/// What a word is after one of its spans closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterSpan {
    /// Read on as usual.
    ReadOn,
    /// The rest of the word is the secret the span's inside still owes.
    Secret,
    /// The span ends the word, and the next word is as this says.
    Next(Pending),
}

/// What follows a span of `kind` whose inside still owes `owed`, as [`units`] says; `goes_on`
/// is whether its word goes on after the close.
const fn after_span(owed: Pending, kind: SpanKind, goes_on: bool) -> AfterSpan {
    let next_word = matches!(owed, Pending::Word | Pending::BasicWord);
    if goes_on {
        if next_word || matches!(owed, Pending::Rest | Pending::AttachedValue) {
            AfterSpan::Secret
        } else {
            AfterSpan::ReadOn
        }
    } else if next_word && !matches!(kind, SpanKind::Substitution) {
        AfterSpan::Next(owed)
    } else {
        AfterSpan::ReadOn
    }
}
