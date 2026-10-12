//! The Bash command as Clef D1 and the decision log see it: trimmed, cut to
//! [`MAX_COMMAND_CHARS`], and with anything that may hold a secret replaced by [`REDACTED`].
//!
//! The rule is position-independent and conservative, so no shell form (a wrapper such as
//! `sudo`, `timeout` or `env -u`, a `bash -c '…'` or `eval "…"` body) can carry a secret past
//! it. Redacting too much only costs Clef some signal; too little leaks.
//!
//! - **Assignments anywhere.** The value of every `NAME=value` (also `NAME+=`, `NAME[i]=` and
//!   `NAME=(…)`), wherever it stands, when `NAME` looks like an environment variable (upper
//!   case, `[A-Z_][A-Z0-9_]*`) or holds a credential word (`decide::redact::CREDENTIAL_WORDS`,
//!   any letter case; the rule is `decide::redact::secret_assignment_prefix`, which the prose
//!   redaction shares), quotes in the name stripped first (`export "API_KEY"=x`). Lower-case
//!   operands such as `dd of=/dev/sda` stay readable: they are what Clef judges.
//! - **Credential flags.** The value of a long option whose name holds a credential word, as
//!   `--token=x` or `--token x`.
//! - **Authorization.** After an `Authorization:` header name, the rest of its word and every
//!   word to the end of its quote or simple command; the word after `Bearer`, and after `Basic`
//!   unless `decide::redact::basic_credential` says it cannot be one.
//! - **URL userinfo.** `https://user:token@host` becomes `https://<redacted>@host`.
//! - **Known tools.** From a tool of `decide::redact::tools` to the end of its simple command,
//!   the values of its secret options (`mysql -pX`, `curl -u user:pass`).
//! - **Quoted text is scanned too**: the inside of `'…'`, `$'…'`, `$( )` and backticks as a
//!   command, the inside of `"…"` with double-quote rules (`'` and `$'` are plain there).
//! - **Comments and heredocs.** An unquoted `#` comment to the end of its line, and every
//!   heredoc body.
//! - **Anything unclosed.** From a quote, substitution or heredoc that is never closed (or nests
//!   past [`MAX_NESTING`]), the rest of the command.
//!
//! The guard's lexer (`shell`) returns words after quote removal and keeps no source positions,
//! so it cannot rewrite the command in place; this scanner reads the same quoting to find each
//! span in the source.

use self::spans::{
    after, char_at, close_ansi, close_backtick, close_double, close_group, close_single, word_end,
};
use super::shell::MAX_NESTING;
use crate::decide::redact::tools::{self, SecretOption, Tool};
use crate::decide::redact::{
    REDACTED, Scheme, authorization_end, basic_credential, is_credential, scheme,
    secret_assignment_prefix, url_userinfo,
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

/// Redacts `text` into `out`, reading it with `quoting` at nesting `depth`.
fn redact_into(text: &str, quoting: Quoting, depth: usize, out: &mut String) -> Scan {
    if depth > MAX_NESTING {
        out.push_str(REDACTED);
        return Scan::Unclosed;
    }
    Scanner {
        text,
        pos: 0,
        out,
        quoting,
        depth,
        pending: Pending::Nothing,
        tool: None,
        heredocs: Vec::new(),
    }
    .run()
}

/// What the words before say about the next one, until a command separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Nothing,
    /// The next word is a secret: a credential option's value, the word after `Bearer`.
    Word,
    /// The next word follows `Basic`: a secret unless `basic_credential` says it cannot be.
    BasicWord,
    /// Every word is a secret: an `Authorization:` header's value.
    Rest,
}

/// Redacts the inside of a closed span; whatever it leaves unclosed is already redacted and
/// ends at the span's own close, so the outer scan carries on.
fn redact_nested(text: &str, quoting: Quoting, depth: usize, out: &mut String) {
    match redact_into(text, quoting, depth.saturating_add(1), out) {
        Scan::Closed | Scan::Unclosed => {}
    }
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
                '\n' => {
                    self.copy_to(next);
                    self.end_command();
                    self.heredoc_bodies()
                }
                ';' | '&' | '|' | '(' | ')' => {
                    self.end_command();
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

    /// At a command separator: what one simple command said about its words ends there.
    fn end_command(&mut self) {
        self.pending = Pending::Nothing;
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
        let secret_prefix = secret_assignment_prefix(raw);
        let Some(end) = end else {
            self.out.push_str(secret_prefix.unwrap_or_default());
            return self.redact_rest();
        };
        let hidden = match pending {
            Pending::Nothing => false,
            Pending::Word => true,
            Pending::BasicWord => basic_credential(raw),
            Pending::Rest => {
                self.pending = Pending::Rest;
                true
            }
        };
        if hidden {
            self.out.push_str(REDACTED);
        } else if let Some(prefix) = secret_prefix {
            self.out.push_str(prefix);
            self.out.push_str(REDACTED);
        } else if !self.authorization(raw) && !self.tool_option(raw) && !self.credential_flag(raw) {
            self.copy_word(raw);
        }
        if self.pending == Pending::Nothing {
            self.pending = match scheme(raw) {
                Some(Scheme::Bearer) => Pending::Word,
                Some(Scheme::Basic) => Pending::BasicWord,
                None => Pending::Nothing,
            };
        }
        if let Some(tool) = tools::known(raw) {
            self.tool = Some(tool);
        }
        self.pos = end;
        Scan::Closed
    }

    /// Writes `raw` redacted when it is a secret option of the command's known tool (its value
    /// in the word, or the next word redacted), and says whether it was one.
    fn tool_option(&mut self, raw: &str) -> bool {
        let Some(option) = self.tool.and_then(|tool| tool.secret_option(raw)) else {
            return false;
        };
        match option {
            SecretOption::Attached(kept) => {
                // `kept` ends within the ASCII option name, so the split exists.
                if let Some((name, _value)) = raw.split_at_checked(kept) {
                    self.out.push_str(name);
                }
                self.out.push_str(REDACTED);
            }
            SecretOption::NextWord => {
                self.out.push_str(raw);
                self.pending = Pending::Word;
            }
        }
        true
    }

    /// Writes `raw` redacted when it holds an `Authorization:` header name outside any quote
    /// (`-HAuthorization:x`): the rest of the word and every word up to the command's end are
    /// its value. One inside a quote is found by the scan of the quote's inside.
    fn authorization(&mut self, raw: &str) -> bool {
        let Some((name, value)) = authorization_end(raw).and_then(|end| raw.split_at_checked(end))
        else {
            return false;
        };
        if name.contains(QUOTING) {
            return false;
        }
        self.out.push_str(name);
        if !value.is_empty() {
            self.out.push_str(REDACTED);
        }
        self.pending = Pending::Rest;
        true
    }

    /// Copies a word with the userinfo of each URL in it (`https://user:token@host`) redacted;
    /// a URL inside a quote is found by the scan of the quote's inside.
    fn copy_word(&mut self, raw: &str) {
        let mut rest = raw;
        while let Some((start, end)) = url_userinfo(rest)
            && let (Some(head), Some(tail)) = (rest.get(..start), rest.get(end..))
            && !head.contains(QUOTING)
        {
            self.out.push_str(head);
            self.out.push_str(REDACTED);
            rest = tail;
        }
        units(rest, self.quoting, self.depth, self.out);
    }

    /// Writes `raw` redacted when it is a credential flag (`--token=x`, or `--token` with the
    /// next word redacted), and says whether it was one.
    fn credential_flag(&mut self, raw: &str) -> bool {
        let Some(flag) = raw.strip_prefix("--") else {
            return false;
        };
        match flag.split_once('=') {
            Some((name, _value)) if is_credential(name) => {
                self.out.push_str("--");
                self.out.push_str(name);
                self.out.push('=');
                self.out.push_str(REDACTED);
                true
            }
            Some(_) => false,
            None => {
                if is_credential(flag) {
                    self.pending = Pending::Word;
                }
                false
            }
        }
    }
}

/// Copies a word whose spans are all closed, scanning the inside of each quote or
/// substitution as text.
fn units(raw: &str, quoting: Quoting, depth: usize, out: &mut String) {
    let mut pos = 0;
    while let Some(c) = char_at(raw, pos) {
        let next = after(pos, c);
        let following = char_at(raw, next);
        let span =
            match (c, following, quoting) {
                ('\\', Some(escaped), _) => {
                    out.push(c);
                    out.push(escaped);
                    pos = after(next, escaped);
                    continue;
                }
                ('\'', _, Quoting::Bare) => {
                    close_single(raw, next).map(|end| ("'", next, end, "'", Quoting::Bare))
                }
                ('"', _, _) => close_double(raw, next, depth)
                    .map(|end| ("\"", next, end, "\"", Quoting::Double)),
                ('`', _, _) => {
                    close_backtick(raw, next).map(|end| ("`", next, end, "`", Quoting::Bare))
                }
                ('$', Some('\''), Quoting::Bare) => {
                    let start = after(next, '\'');
                    close_ansi(raw, start).map(|end| ("$'", start, end, "'", Quoting::Bare))
                }
                ('$', Some(open @ ('(' | '{')), _) => {
                    let start = after(next, open);
                    let close = if open == '(' { ')' } else { '}' };
                    close_group(raw, start, close, depth).map(|end| {
                        (
                            if open == '(' { "$(" } else { "${" },
                            start,
                            end,
                            if open == '(' { ")" } else { "}" },
                            Quoting::Bare,
                        )
                    })
                }
                ('(', _, Quoting::Bare) => close_group(raw, next, ')', depth)
                    .map(|end| ("(", next, end, ")", Quoting::Bare)),
                _ => {
                    out.push(c);
                    pos = next;
                    continue;
                }
            };
        // A span `word_end` found closed closes here too; should one not, redact the rest.
        let Some((open, start, end, close, inner_quoting)) = span else {
            out.push_str(REDACTED);
            return;
        };
        let inner_end = end.saturating_sub(close.len());
        out.push_str(open);
        redact_nested(
            raw.get(start..inner_end).unwrap_or_default(),
            inner_quoting,
            depth,
            out,
        );
        out.push_str(close);
        pos = end;
    }
}

mod spans;

#[cfg(test)]
mod tests;
