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
//! - **Each word judged as bash reads it** (`word`): its text with quotes removed and line
//!   continuations dropped is judged whole by `decide::redact::judge`, which decides what it
//!   owes the next word (`"X-Api-Key: " secret`, `Bearer\` then a newline and `tok`); an
//!   assignment (`x="Bearer" dropdb`) owes nothing. A word whose rewrite still holds a secret
//!   when read back that way is hidden whole. An unquoted line continuation between words is a
//!   blank.
//! - **Comments and heredocs** (`heredoc`). An unquoted `#` comment to the end of its line,
//!   and every heredoc body.
//! - **Anything unclosed.** From a quote, substitution or heredoc that is never closed (or nests
//!   past [`MAX_NESTING`]), the rest of the command.
//!
//! The guard's lexer (`shell`) returns words after quote removal and keeps no source positions,
//! so it cannot rewrite the command in place; this scanner reads the same quoting to find each
//! span in the source.

use self::heredoc::Heredoc;
use self::spans::{after, char_at, word_end};
use super::shell::MAX_NESTING;
use crate::decide::redact::rules::{is_pair, secret_assignment_prefix};
use crate::decide::redact::tools::Tool;
use crate::decide::redact::{REDACTED, basic_credential};
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
}

mod heredoc;
mod spans;
mod word;

#[cfg(test)]
mod tests;
