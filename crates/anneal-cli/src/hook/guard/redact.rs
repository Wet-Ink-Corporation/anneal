//! The Bash command as Clef D1 and the decision log see it: trimmed, cut to
//! [`MAX_COMMAND_CHARS`], and with the values of env assignments and credential flags replaced
//! by [`REDACTED`].
//!
//! Redacted: the value of every assignment in command position (`FOO=bar cmd`, after `;`, `&&`,
//! `|`, `(`, a newline or a keyword such as `then`), of every assignment after `env` (and its
//! options) and after the declaration builtins (`export`, `declare`, `typeset`, `local`,
//! `readonly`), and of every long option whose name holds a credential word
//! ([`CREDENTIAL_WORDS`]), whether given as `--token=x` or `--token x`. Arguments that only look
//! like assignments (`dd of=/dev/sda`, `make CC=gcc`) are kept: they are what Clef judges.
//!
//! The guard's lexer (`shell`) returns words after quote removal and keeps no source positions,
//! so it cannot rewrite the command in place. This scanner reads the same quoting (`'…'`,
//! `"…"`, `$'…'`, backslashes, `$( )`, `${ }`, backticks) and the same metacharacters to find
//! each word's span in the source, and replaces only the value's span. What it cannot close (an
//! unclosed quote, nesting past [`MAX_NESTING`]) runs to the end of the command, so a value is
//! redacted too much rather than too little.

use super::shell::MAX_NESTING;

/// What a redacted value is replaced with.
pub(crate) const REDACTED: &str = "<redacted>";

/// Most characters of the command kept, after redaction; a longer one ends in `…`. The same as
/// `decide::state::MAX_STRING_CHARS`, so the compact state does not cut it again.
pub(crate) const MAX_COMMAND_CHARS: usize = crate::decide::state::MAX_STRING_CHARS;

/// Words a credential flag's name holds, compared in ASCII lower case.
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

/// Builtins whose `NAME=value` arguments are assignments.
const DECLARATIONS: [&str; 5] = ["export", "declare", "typeset", "local", "readonly"];

/// Reserved words after which a command (and its assignments) can still follow.
const KEYWORDS: [&str; 10] = [
    "!", "{", "if", "then", "else", "elif", "do", "while", "until", "time",
];

/// `command` trimmed, with its secret values redacted, cut to [`MAX_COMMAND_CHARS`].
pub(crate) fn redact(command: &str) -> String {
    let mut redactor = Redactor::new(command);
    redactor.run();
    cut(redactor.out.trim())
}

/// Where the next word stands in its simple command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    /// Before the program: assignments and keywords.
    Command,
    /// After `env`: options and assignments, then the program.
    Env,
    /// After a declaration builtin: every `NAME=value` is an assignment.
    Declaration,
    /// The program's arguments.
    Arguments,
}

/// The scan: the source, the cursor, the output so far and what the next word is.
#[derive(Debug)]
struct Redactor<'a> {
    text: &'a str,
    pos: usize,
    out: String,
    position: Position,
    /// The next word is a credential flag's value.
    secret_next: bool,
    /// The next word is a redirection's target, which is neither a program nor an assignment.
    target_next: bool,
}

impl<'a> Redactor<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            pos: 0,
            out: String::with_capacity(text.len()),
            position: Position::Command,
            secret_next: false,
            target_next: false,
        }
    }

    fn run(&mut self) {
        while let Some(c) = char_at(self.text, self.pos) {
            let next = self.pos.saturating_add(c.len_utf8());
            match c {
                ' ' | '\t' => self.copy_to(next),
                '\n' | ';' | '&' | '|' | '(' | ')' => {
                    self.copy_to(next);
                    self.position = Position::Command;
                    self.secret_next = false;
                    self.target_next = false;
                }
                '<' | '>' => {
                    self.copy_to(next);
                    self.secret_next = false;
                    self.target_next = true;
                }
                _ => {
                    let end = word_end(self.text, self.pos, 0);
                    // Every position is a char boundary within `text`, so the span exists; an
                    // empty word would only drop text, never a redaction.
                    let word = self.text.get(self.pos..end).unwrap_or_default();
                    self.word(word);
                    self.pos = end;
                }
            }
        }
    }

    /// Copies the source up to `end` (a char boundary past `pos`) unchanged.
    fn copy_to(&mut self, end: usize) {
        self.out
            .push_str(self.text.get(self.pos..end).unwrap_or_default());
        self.pos = end;
    }

    /// Writes `word`, redacted where its place in the command says so.
    fn word(&mut self, word: &str) {
        if self.target_next {
            self.target_next = false;
            self.out.push_str(word);
            return;
        }
        if self.secret_next {
            self.secret_next = false;
            self.out.push_str(REDACTED);
            return;
        }
        let assigns = self.position != Position::Arguments;
        if let Some(name) = assignment_name(word).filter(|_| assigns) {
            self.out.push_str(name);
            self.out.push('=');
            self.out.push_str(REDACTED);
            return;
        }
        match self.position {
            Position::Env if word.starts_with('-') => self.out.push_str(word),
            Position::Command | Position::Env => self.program(word),
            Position::Declaration | Position::Arguments => self.argument(word),
        }
    }

    /// Writes a word in program position and moves past it.
    fn program(&mut self, word: &str) {
        self.position = if KEYWORDS.contains(&word) {
            Position::Command
        } else if DECLARATIONS.contains(&word) {
            Position::Declaration
        } else if word == "env" {
            Position::Env
        } else {
            Position::Arguments
        };
        self.argument(word);
    }

    /// Writes an argument, redacting a credential flag's value.
    fn argument(&mut self, word: &str) {
        let Some(flag) = word.strip_prefix("--") else {
            self.out.push_str(word);
            return;
        };
        match flag.split_once('=') {
            Some((name, _value)) if is_credential(name) => {
                self.out.push_str("--");
                self.out.push_str(name);
                self.out.push('=');
                self.out.push_str(REDACTED);
            }
            Some(_) => self.out.push_str(word),
            None => {
                self.secret_next = is_credential(flag);
                self.out.push_str(word);
            }
        }
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

/// Where the word starting at `start` ends: the first unquoted blank or metacharacter outside
/// any quote or substitution, else the end of `text`.
fn word_end(text: &str, start: usize, depth: usize) -> usize {
    let mut pos = start;
    while let Some(c) = char_at(text, pos) {
        if matches!(
            c,
            ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>'
        ) {
            return pos;
        }
        pos = skip_unit(text, pos, c, depth);
    }
    pos
}

/// Skips the quoting or substitution starting with `c` at `pos` (or the plain `c`), and returns
/// where it ends.
fn skip_unit(text: &str, pos: usize, c: char, depth: usize) -> usize {
    let next = after(pos, c);
    if depth > MAX_NESTING {
        return text.len();
    }
    match (c, char_at(text, next)) {
        ('\\', Some(escaped)) => after(next, escaped),
        ('\'', _) => close_single(text, next),
        ('"', _) => close_double(text, next, depth),
        ('`', _) => close_backtick(text, next),
        ('$', Some('\'')) => close_ansi(text, after(next, '\'')),
        ('$', Some('(')) => close_group(text, after(next, '('), ')', depth.saturating_add(1)),
        ('$', Some('{')) => close_group(text, after(next, '{'), '}', depth.saturating_add(1)),
        _ => next,
    }
}

/// Past the `'` closing a single-quoted string that starts at `pos`.
fn close_single(text: &str, pos: usize) -> usize {
    text.get(pos..)
        .and_then(|rest| rest.find('\''))
        .map_or(text.len(), |at| pos.saturating_add(at).saturating_add(1))
}

/// Past the `'` closing an ANSI-C string (`$'…'`, backslash escapes) that starts at `pos`.
fn close_ansi(text: &str, mut pos: usize) -> usize {
    while let Some(c) = char_at(text, pos) {
        match c {
            '\'' => return after(pos, c),
            '\\' => pos = skip_escape(text, pos),
            _ => pos = after(pos, c),
        }
    }
    pos
}

/// Past the backtick closing a substitution that starts at `pos`.
fn close_backtick(text: &str, mut pos: usize) -> usize {
    while let Some(c) = char_at(text, pos) {
        match c {
            '`' => return after(pos, c),
            '\\' => pos = skip_escape(text, pos),
            _ => pos = after(pos, c),
        }
    }
    pos
}

/// Past the `"` closing a double-quoted string that starts at `pos`; substitutions inside it
/// are skipped whole.
fn close_double(text: &str, mut pos: usize, depth: usize) -> usize {
    while let Some(c) = char_at(text, pos) {
        match c {
            '"' => return after(pos, c),
            '\\' => pos = skip_escape(text, pos),
            '$' | '`' => pos = skip_unit(text, pos, c, depth),
            _ => pos = after(pos, c),
        }
    }
    pos
}

/// Past the `close` ending a `$( )`, `( )` or `${ }` group whose contents start at `pos`.
fn close_group(text: &str, mut pos: usize, close: char, depth: usize) -> usize {
    while let Some(c) = char_at(text, pos) {
        if c == close {
            return after(pos, c);
        }
        pos = if c == '(' {
            close_group(text, after(pos, c), ')', depth.saturating_add(1))
        } else {
            skip_unit(text, pos, c, depth)
        };
        if depth > MAX_NESTING {
            return text.len();
        }
    }
    pos
}

/// Past a backslash at `pos` and the character it escapes.
fn skip_escape(text: &str, pos: usize) -> usize {
    let next = after(pos, '\\');
    char_at(text, next).map_or(next, |escaped| after(next, escaped))
}

/// The `NAME` (or `NAME+`) of `word` when it is an assignment, `NAME=value` or `NAME+=value`.
fn assignment_name(word: &str) -> Option<&str> {
    let (name, _value) = word.split_once('=')?;
    is_name(name.strip_suffix('+').unwrap_or(name)).then_some(name)
}

/// A shell identifier.
fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether a long option's name holds a credential word.
fn is_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    CREDENTIAL_WORDS.iter().any(|word| name.contains(word))
}

/// `text` cut to [`MAX_COMMAND_CHARS`] characters plus `…` when longer.
fn cut(text: &str) -> String {
    match text.char_indices().nth(MAX_COMMAND_CHARS) {
        Some((end, _)) => {
            let kept = text.get(..end).unwrap_or(text);
            format!("{kept}…")
        }
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests;
