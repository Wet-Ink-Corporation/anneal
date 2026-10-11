//! The Bash command as Clef D1 and the decision log see it: trimmed, cut to
//! [`MAX_COMMAND_CHARS`], and with anything that may hold a secret replaced by [`REDACTED`].
//!
//! The rule is position-independent and conservative, so no shell form (a wrapper such as
//! `sudo`, `timeout` or `env -u`, a `bash -c '…'` or `eval "…"` body) can carry a secret past
//! it. Redacting too much only costs Clef some signal; too little leaks.
//!
//! - **Assignments anywhere.** The value of every `NAME=value` (also `NAME+=`, `NAME[i]=` and
//!   `NAME=(…)`), wherever it stands, when `NAME` looks like an environment variable (upper
//!   case, `[A-Z_][A-Z0-9_]*`) or holds a credential word ([`CREDENTIAL_WORDS`], any letter
//!   case). Lower-case operands such as `dd of=/dev/sda` stay readable: they are what Clef
//!   judges.
//! - **Credential flags.** The value of a long option whose name holds a credential word, as
//!   `--token=x` or `--token x`.
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

use super::shell::MAX_NESTING;
use crate::decide::state::cut_chars;

/// What a redacted value is replaced with.
pub(crate) const REDACTED: &str = "<redacted>";

/// Most characters of the command kept, after redaction; a longer one ends in `…`. The same as
/// `decide::state::MAX_STRING_CHARS`, so the compact state does not cut it again.
pub(crate) const MAX_COMMAND_CHARS: usize = crate::decide::state::MAX_STRING_CHARS;

/// Words a credential name holds, compared in ASCII lower case.
pub(crate) const CREDENTIAL_WORDS: [&str; 10] = [
    "token",
    "password",
    "passwd",
    "secret",
    "apikey",
    "api-key",
    "api_key",
    "credential",
    "private-key",
    "private_key",
];

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
        secret_next: false,
        heredocs: Vec::new(),
    }
    .run()
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
    /// The next word is a credential flag's value.
    secret_next: bool,
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
                    self.secret_next = false;
                    self.heredoc_bodies()
                }
                ';' | '&' | '|' | '(' | ')' => {
                    self.secret_next = false;
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
        let secret_value = std::mem::take(&mut self.secret_next);
        let secret_prefix = secret_assignment_prefix(raw);
        let Some(end) = end else {
            self.out.push_str(secret_prefix.unwrap_or_default());
            return self.redact_rest();
        };
        if secret_value {
            self.out.push_str(REDACTED);
        } else if let Some(prefix) = secret_prefix {
            self.out.push_str(prefix);
            self.out.push_str(REDACTED);
        } else if !self.credential_flag(raw) {
            units(raw, self.quoting, self.depth, self.out);
        }
        self.pos = end;
        Scan::Closed
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
                self.secret_next = is_credential(flag);
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

/// The char at byte `pos` of `text`, if any.
fn char_at(text: &str, pos: usize) -> Option<char> {
    text.get(pos..).and_then(|rest| rest.chars().next())
}

/// The byte just past `c` at `pos`.
fn after(pos: usize, c: char) -> usize {
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
fn word_end(text: &str, start: usize, quoting: Quoting, depth: usize) -> Option<usize> {
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
fn close_single(text: &str, pos: usize) -> Option<usize> {
    let at = text.get(pos..)?.find('\'')?;
    Some(pos.saturating_add(at).saturating_add(1))
}

/// Past the `'` closing an ANSI-C string (`$'…'`, backslash escapes) whose inside starts at
/// `pos`.
fn close_ansi(text: &str, mut pos: usize) -> Option<usize> {
    loop {
        match char_at(text, pos)? {
            '\'' => return Some(after(pos, '\'')),
            '\\' => pos = skip_escape(text, pos),
            c => pos = after(pos, c),
        }
    }
}

/// Past the backtick closing a substitution whose inside starts at `pos`.
fn close_backtick(text: &str, mut pos: usize) -> Option<usize> {
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
fn close_double(text: &str, mut pos: usize, depth: usize) -> Option<usize> {
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
fn close_group(text: &str, mut pos: usize, close: char, depth: usize) -> Option<usize> {
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

/// `NAME=`, `NAME+=`, `NAME[…]=` or `NAME[…]+=` at the start of `word`, through the `=`, when
/// `NAME` is an environment-style name or holds a credential word.
fn secret_assignment_prefix(word: &str) -> Option<&str> {
    let name_len = word
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(word.len());
    let name = word.get(..name_len)?;
    if !(is_name(name) && (is_env_name(name) || is_credential(name))) {
        return None;
    }
    let mut end = name_len;
    if word.get(end..)?.starts_with('[') {
        let close = word.get(end..)?.find(']')?;
        end = end.saturating_add(close).saturating_add(1);
    }
    let rest = word.get(end..)?;
    let equals = if rest.starts_with("+=") {
        2
    } else if rest.starts_with('=') {
        1
    } else {
        return None;
    };
    word.get(..end.saturating_add(equals))
}

/// A shell identifier.
fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// An environment-style name: `[A-Z_][A-Z0-9_]*`.
fn is_env_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Whether a name holds a credential word, in any letter case.
fn is_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    CREDENTIAL_WORDS.iter().any(|word| name.contains(word))
}

#[cfg(test)]
mod tests;
