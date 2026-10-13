//! The Bash command as Clef D1 and the decision log see it: trimmed, cut to
//! [`MAX_COMMAND_CHARS`], and with anything that may hold a secret replaced by [`REDACTED`].
//!
//! The rule is position-independent and conservative, so no shell form (a wrapper such as
//! `sudo`, `timeout` or `env -u`, a `bash -c '…'` or `eval "…"` body) can carry a secret past
//! it. Redacting too much only costs Clef some signal; too little leaks.
//!
//! - **The shared word rules** (`decide::redact::rules::find_secret`), wherever a word stands:
//!   the value of an environment-style or credential-named assignment (`TOKEN=x`,
//!   `"API_KEY"=x`; lower-case operands such as `dd of=/dev/sda` stay readable, they are what
//!   Clef judges), of a credential-named key inside a word (`--from-literal=password=x`,
//!   `{"password":"x"}`), of a credential-named option or configuration key (`--password x`,
//!   `-storepass x`, `user.password x`; not `--password-stdin`), and of
//!   a known tool's secret option (`mysql -pX`, `docker login -p x`). A word's secret is taken
//!   only when what stays readable before it closes every quote it opens; otherwise the inside
//!   of the quote is scanned and finds it there.
//! - **Headers.** After a credential-named header name (`Authorization:`, `X-Api-Key:`,
//!   `Cookie:`), every word to the end of its simple command or quote. A header-shaped name
//!   with its value attached (`Cookie:a=1`, `X-Api-Key:k`, but also `db-password:/p`) hides
//!   only that value. When the value attached to any credential name is the scheme word
//!   (`X-Auth-Token:Bearer xyz`, `Token:Bearer xyz`), the scheme's credential in the next word
//!   goes as well. Inside a quote or substitution, the
//!   `name=value` words after a `;` that ends either are hidden too (`"Cookie: a=1; b=2"`,
//!   `"Cookie:a=1; sid=\"S\""`), but no other word, nor a pair whose value holds a
//!   substitution, escape, blank, `;` or a quote it does not close, so no header hides a
//!   command; the word after `Bearer`, and after `Basic` unless
//!   `decide::redact::basic_credential` says it cannot be one.
//! - **Subcommand arguments.** The word after `vault login` (`decide::redact::tools`).
//! - **URL userinfo and tokens.** `https://user:token@host` becomes `https://<redacted>@host`;
//!   an unquoted word that looks like a token (`decide::redact::looks_like_token`) goes whole.
//! - **Quoted text is scanned too**: the inside of `'…'`, `$'…'`, `$( )` and backticks as a
//!   command, the inside of `"…"` with double-quote rules (`'` and `$'` are plain there).
//!   When the inside ends still owing a secret, the rest of its word goes if the word goes on
//!   after the close (`"X-Api-Key:"secret`), and a quote ending its word on a scheme word
//!   hides the next word (`"X-Auth-Token:Bearer" xyz`); a substitution passes nothing on.
//! - **Line continuations.** `\` and a newline are a blank, so the secret a word says comes
//!   next is the word after them (`--password \` then `hunter2`).
//! - **Comments and heredocs.** An unquoted `#` comment to the end of its line, and every
//!   heredoc body.
//! - **Anything unclosed.** From a quote, substitution or heredoc that is never closed (or nests
//!   past [`MAX_NESTING`]), the rest of the command.
//!
//! The guard's lexer (`shell`) returns words after quote removal and keeps no source positions,
//! so it cannot rewrite the command in place; this scanner reads the same quoting to find each
//! span in the source.

use self::spans::{Unit, after, char_at, closed, unit_at, word_end};
use super::shell::MAX_NESTING;
use crate::decide::redact::rules::{is_pair, secret_assignment_prefix};
use crate::decide::redact::tools::{self, Tool};
use crate::decide::redact::{
    REDACTED, Scheme, Secret, Then, basic_credential, find_secret, looks_like_token, scheme,
    split_punctuation, url_userinfo,
};
use crate::decide::state::cut_chars;

/// Characters that open a quote or substitution: a word part holding none is plain text.
const QUOTING: [char; 6] = ['\'', '"', '`', '$', '\\', '('];

/// Most characters of the command kept, after redaction; a longer one ends in `…`. The same as
/// `decide::state::MAX_STRING_CHARS`, so the compact state does not cut it again.
pub(crate) const MAX_COMMAND_CHARS: usize = crate::decide::state::MAX_STRING_CHARS;

/// `command` trimmed, with its secrets redacted, cut to [`MAX_COMMAND_CHARS`].
pub(crate) fn redact(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    redact_into(command, Quoting::Bare, 0, &mut out);
    cut_chars(out.trim(), MAX_COMMAND_CHARS)
}

/// Whether a scan read its text to the end with every span closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    Closed,
    /// Something was not closed; the rest is [`REDACTED`].
    Unclosed,
}

/// The quoting the text being scanned sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quoting {
    /// Unquoted, or inside `'…'`, `$'…'`, `$( )` or backticks: read as a command.
    Bare,
    /// Inside `"…"`: `'` and `$'` are plain characters.
    Double,
}

/// A heredoc whose body starts after the current line.
#[derive(Debug)]
struct Heredoc {
    delimiter: String,
    /// `<<-`: leading tabs are stripped from the closing line.
    strip_tabs: bool,
}

/// Redacts `text` into `out`, reading it with `quoting` at nesting `depth`. Returns what its
/// last words say about the words after it: [`Pending::Nothing`] when something was not
/// closed, since the rest is already [`REDACTED`].
fn redact_into(text: &str, quoting: Quoting, depth: usize, out: &mut String) -> Pending {
    if depth > MAX_NESTING {
        out.push_str(REDACTED);
        return Pending::Nothing;
    }
    let mut scanner = Scanner {
        text,
        pos: 0,
        out,
        quoting,
        depth,
        pending: Pending::Nothing,
        tool: None,
        heredocs: Vec::new(),
    };
    match scanner.run() {
        Scan::Closed => scanner.pending,
        Scan::Unclosed => Pending::Nothing,
    }
}

/// What the words before say about the next one, until a command separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Nothing,
    /// The next word is a secret: a credential option's value, the word after `Bearer`.
    Word,
    /// The next word follows `Basic`: a secret unless `basic_credential` says it cannot be.
    BasicWord,
    /// Every word is a secret, to the end of the simple command: a header's value.
    Rest,
    /// The word before was a header with its value attached (`Cookie:a=1`): the next word is
    /// read as usual, and only a `;` inside a quote or substitution leads on to the header's
    /// other cookie pairs ([`Pending::Pairs`]).
    AttachedValue,
    /// After a `;` that ended a header's value inside a quote or substitution: each
    /// `name=value` word is a secret (the other pairs of `"Cookie: a=1; b=2"`); the first other
    /// word is read as usual, so it ends this.
    Pairs,
}

/// Redacts the inside of a closed span; whatever it leaves unclosed is already redacted and
/// ends at the span's own close, so the outer scan carries on. Returns what the inside's last
/// words say about the text after the close ([`redact_into`]).
fn redact_nested(text: &str, quoting: Quoting, depth: usize, out: &mut String) -> Pending {
    redact_into(text, quoting, depth.saturating_add(1), out)
}

/// The scan of one text: the cursor, the output, and what the next word or line is.
#[derive(Debug)]
struct Scanner<'t, 'o> {
    text: &'t str,
    pos: usize,
    out: &'o mut String,
    quoting: Quoting,
    depth: usize,
    /// Whether the next word, or every word, is a secret.
    pending: Pending,
    /// The known tool this simple command named, whose secret options are redacted.
    tool: Option<Tool>,
    /// Heredocs opened on the current line.
    heredocs: Vec<Heredoc>,
}

impl Scanner<'_, '_> {
    fn run(&mut self) -> Scan {
        while let Some(c) = char_at(self.text, self.pos) {
            let next = after(self.pos, c);
            let scan = match c {
                ' ' | '\t' => self.copy_to(next),
                // A line continuation is a blank: it ends no command and is no word, so it
                // leaves a pending secret for the word after it.
                '\\' if char_at(self.text, next) == Some('\n') => self.copy_to(after(next, '\n')),
                '\n' => {
                    self.copy_to(next);
                    self.end_command(c);
                    self.heredoc_bodies()
                }
                ';' | '&' | '|' | '(' | ')' => {
                    self.end_command(c);
                    self.copy_to(next)
                }
                '<' | '>' => self.redirection(c),
                '#' => self.comment(),
                _ => self.word(),
            };
            if scan == Scan::Unclosed {
                return Scan::Unclosed;
            }
        }
        if self.heredocs.is_empty() {
            Scan::Closed
        } else {
            self.out.push_str(REDACTED);
            Scan::Unclosed
        }
    }

    /// At the command separator `separator`: what one simple command said about its words ends
    /// there, so no secret rule hides a command word. Inside a quote or substitution a `;` after
    /// a header's value may instead sit between its pairs (`"Cookie: a=1; b=2"`), so the
    /// `name=value` words after it stay hidden ([`Pending::Pairs`]).
    fn end_command(&mut self, separator: char) {
        self.pending = match self.pending {
            Pending::Rest | Pending::AttachedValue | Pending::Pairs
                if self.depth > 0 && separator == ';' =>
            {
                Pending::Pairs
            }
            Pending::Nothing
            | Pending::Word
            | Pending::BasicWord
            | Pending::Rest
            | Pending::AttachedValue
            | Pending::Pairs => Pending::Nothing,
        };
        self.tool = None;
    }

    /// Copies the source up to `end` (a char boundary past `pos`) unchanged.
    fn copy_to(&mut self, end: usize) -> Scan {
        self.out
            .push_str(self.text.get(self.pos..end).unwrap_or_default());
        self.pos = end;
        Scan::Closed
    }

    /// Redacts from `pos` to the end of the text.
    fn redact_rest(&mut self) -> Scan {
        self.out.push_str(REDACTED);
        self.pos = self.text.len();
        Scan::Unclosed
    }

    /// A run of `<` or `>`; exactly `<<` (or `<<-`) outside double quotes opens a heredoc.
    fn redirection(&mut self, c: char) -> Scan {
        let run = self
            .text
            .get(self.pos..)
            .map_or(0, |rest| rest.chars().take_while(|&d| d == c).count());
        let end = self.pos.saturating_add(run);
        if c == '<' && run == 2 && self.quoting == Quoting::Bare {
            self.copy_to(end);
            self.heredoc()
        } else {
            self.copy_to(end)
        }
    }

    /// After `<<`: an optional `-`, blanks and the delimiter word, queued for the next line.
    fn heredoc(&mut self) -> Scan {
        let strip_tabs = char_at(self.text, self.pos) == Some('-');
        if strip_tabs {
            self.copy_to(after(self.pos, '-'));
        }
        while let Some(blank @ (' ' | '\t')) = char_at(self.text, self.pos) {
            self.copy_to(after(self.pos, blank));
        }
        let Some(end) = word_end(self.text, self.pos, self.quoting, self.depth) else {
            return self.redact_rest();
        };
        let raw = self.text.get(self.pos..end).unwrap_or_default();
        let delimiter = raw
            .chars()
            .filter(|c| !matches!(c, '\'' | '"' | '\\'))
            .collect();
        self.heredocs.push(Heredoc {
            delimiter,
            strip_tabs,
        });
        self.copy_to(end)
    }

    /// At the start of a line: each queued heredoc's body becomes [`REDACTED`], its closing
    /// line is kept.
    fn heredoc_bodies(&mut self) -> Scan {
        for heredoc in std::mem::take(&mut self.heredocs) {
            let mut line_start = self.pos;
            loop {
                let rest = self.text.get(line_start..).unwrap_or_default();
                let line_len = rest.find('\n').unwrap_or(rest.len());
                let line = rest.get(..line_len).unwrap_or_default();
                let closing = if heredoc.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if closing == heredoc.delimiter {
                    if line_start > self.pos {
                        self.out.push_str(REDACTED);
                        self.out.push('\n');
                    }
                    self.pos = line_start;
                    let newline = usize::from(line_len < rest.len());
                    self.copy_to(line_start.saturating_add(line_len).saturating_add(newline));
                    break;
                }
                if line_len == rest.len() {
                    return self.redact_rest();
                }
                line_start = line_start.saturating_add(line_len).saturating_add(1);
            }
        }
        Scan::Closed
    }

    /// `#` at the start of a word: a comment to the end of the line.
    fn comment(&mut self) -> Scan {
        let rest = self.text.get(self.pos..).unwrap_or_default();
        let len = rest.find('\n').unwrap_or(rest.len());
        self.out.push('#');
        if len > 1 {
            self.out.push_str(REDACTED);
        }
        self.pos = self.pos.saturating_add(len);
        Scan::Closed
    }

    /// One word, redacted as the module documentation says.
    fn word(&mut self) -> Scan {
        let end = word_end(self.text, self.pos, self.quoting, self.depth);
        // Every position is a char boundary within `text`, so the span exists.
        let raw = self
            .text
            .get(self.pos..end.unwrap_or(self.text.len()))
            .unwrap_or_default();
        let pending = std::mem::replace(&mut self.pending, Pending::Nothing);
        let Some(end) = end else {
            self.out
                .push_str(secret_assignment_prefix(raw).unwrap_or_default());
            return self.redact_rest();
        };
        let hidden = match pending {
            Pending::Nothing | Pending::AttachedValue => false,
            Pending::Word => true,
            Pending::BasicWord => basic_credential(raw),
            Pending::Rest => {
                self.pending = Pending::Rest;
                true
            }
            Pending::Pairs => {
                let pair = is_pair(raw);
                if pair {
                    self.pending = Pending::Pairs;
                }
                pair
            }
        };
        if hidden {
            self.out.push_str(REDACTED);
        } else {
            self.readable_word(raw);
        }
        self.pos = end;
        Scan::Closed
    }

    /// A word not hidden by the words before it: its own secret, a token, or the word with any
    /// URL userinfo redacted; then what it says about the next words.
    fn readable_word(&mut self, raw: &str) {
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

/// The punctuation around an unquoted word that looks like a token (`ghp_…`), so the token
/// goes whole; a quoted one is found by the scan of the quote's inside.
fn plain_token(raw: &str) -> Option<(&str, &str)> {
    if raw.contains(QUOTING) {
        return None;
    }
    let (lead, core, trail) = split_punctuation(raw);
    looks_like_token(core).then_some((lead, trail))
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
        let (open, start, end, close, inner_quoting) = match unit_at(raw, pos, quoting, depth) {
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
        out.push_str(open);
        let inner = raw.get(start..end.saturating_sub(close.len()));
        let owed = redact_nested(inner.unwrap_or_default(), inner_quoting, depth, out);
        out.push_str(close);
        pos = end;
        match after_span(owed, open, pos < raw.len()) {
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

/// What follows a span opened by `open` whose inside still owes `owed`, as [`units`] says;
/// `goes_on` is whether its word goes on after the close.
fn after_span(owed: Pending, open: &str, goes_on: bool) -> AfterSpan {
    let next_word = matches!(owed, Pending::Word | Pending::BasicWord);
    if goes_on {
        if next_word || matches!(owed, Pending::Rest | Pending::AttachedValue) {
            AfterSpan::Secret
        } else {
            AfterSpan::ReadOn
        }
    } else if next_word && matches!(open, "'" | "\"" | "$'") {
        AfterSpan::Next(owed)
    } else {
        AfterSpan::ReadOn
    }
}

mod spans;

#[cfg(test)]
mod tests;
