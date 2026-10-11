//! A POSIX-ish shell lexer: enough of `sh` to see every simple command a Bash tool call runs.
//!
//! [`parse`] removes quotes (single, double, ANSI-C `$'...'`) and backslash escapes, splits at
//! `&&`, `||`, `;`, `|`, `&` and newlines, and flattens `$( )`, backticks, `( )` subshells,
//! `<( )` process substitutions and the substitutions in unquoted heredoc bodies into the same
//! list, so a command cannot hide inside another. Heredoc bodies are data. Expansions are not
//! performed: a word that has one is marked [`Shape::Dynamic`], one with an unquoted glob,
//! brace or leading `~` [`Shape::Pattern`].
//!
//! Commands and redirections are read here; words (quotes, escapes, `$`) in [`word`], heredoc
//! bodies in [`heredoc`].

mod heredoc;
mod word;

use std::mem;

/// How deep `$( )`, backticks, `( )` and `${ }` may nest before a command is refused (H-13).
pub(crate) const MAX_NESTING: usize = 32;

/// A shell word after quote removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    /// The word's text with quotes and escapes removed; an expansion keeps its source text.
    pub(crate) text: String,
    /// How far the text is from what the shell passes on.
    pub(crate) shape: Shape,
    /// Whether any part of the word was quoted or escaped.
    pub(crate) quoted: bool,
}

/// How far a word's text is from what the shell passes on, from exact to unknowable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Shape {
    /// No expansion: the text is the argument.
    Literal,
    /// An unquoted glob (`*`, `?`, `[`), brace or leading `~`: the text is a pattern.
    Pattern,
    /// A variable, command substitution or arithmetic: the value is only known at run time.
    Dynamic,
}

impl Word {
    const fn new() -> Self {
        Self {
            text: String::new(),
            shape: Shape::Literal,
            quoted: false,
        }
    }

    fn mark(&mut self, shape: Shape) {
        self.shape = self.shape.max(shape);
    }

    /// Whether the text is exactly what runs.
    pub(crate) fn is_literal(&self) -> bool {
        self.shape == Shape::Literal
    }

    /// Whether the word exists at all: `''` does, an unquoted nothing does not.
    fn is_present(&self) -> bool {
        !self.text.is_empty() || self.quoted || !self.is_literal()
    }

    /// An unquoted run of digits, which before `<` or `>` names a file descriptor.
    fn is_fd(&self) -> bool {
        !self.quoted
            && self.is_literal()
            && !self.text.is_empty()
            && self.text.chars().all(|c| c.is_ascii_digit())
    }
}

/// A simple command: its words and the files its redirections write.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SimpleCommand {
    /// The words, program first.
    pub(crate) words: Vec<Word>,
    /// The targets of `>`, `>>`, `>|`, `&>`, `<>` and `>&file`.
    pub(crate) writes: Vec<Word>,
}

/// Why a command could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ParseError {
    /// A quote or substitution is never closed.
    #[error("an unclosed {what}")]
    Unclosed {
        /// What was opened.
        what: &'static str,
    },
    /// Substitutions or subshells nest deeper than [`MAX_NESTING`].
    #[error("substitutions or subshells nested deeper than {MAX_NESTING}")]
    TooDeep,
    /// A redirection operator with no word after it.
    #[error("a redirection with no target")]
    MissingRedirectTarget,
}

/// Lexes `text` into every simple command it runs, nested ones included.
///
/// # Errors
/// [`ParseError`] when a quote or substitution is not closed, nesting is too deep, or a
/// redirection has no target: the shell would refuse it or read it differently.
pub(crate) fn parse(text: &str) -> Result<Vec<SimpleCommand>, ParseError> {
    let mut lexer = Lexer::new(text);
    lexer.list(Close::End, 0)?;
    Ok(lexer.commands)
}

/// What ends the command list being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Close {
    /// The end of the input.
    End,
    /// A `)`; the text says what opened it.
    Paren(&'static str),
    /// A closing backtick.
    Backtick,
}

/// Whether `$` inside double quotes, where `$'` and `$"` are not quoting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quoting {
    Bare,
    Double,
}

/// What a redirection operator does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redirect {
    /// `>`, `>>`, `>|`, `&>`, `&>>`, `<>`: the target is written.
    Write,
    /// `>&`: a file when the target is not a descriptor.
    Duplicate,
    /// `<`, `<&`, `<<<`: the target is read.
    Read,
    /// `<<` or `<<-`.
    Heredoc { strip_tabs: bool },
    /// `<(` or `>(`.
    Process,
}

/// A heredoc whose body starts after the next newline.
#[derive(Debug)]
struct Heredoc {
    delimiter: String,
    /// An unquoted delimiter: the body's substitutions run.
    expands: bool,
    strip_tabs: bool,
}

/// The lexer's state: the input, the cursor, the commands read so far and the heredocs whose
/// bodies start at the next newline.
#[derive(Debug)]
struct Lexer {
    chars: Vec<char>,
    pos: usize,
    commands: Vec<SimpleCommand>,
    heredocs: Vec<Heredoc>,
}

/// Characters that end an unquoted word.
const fn is_meta(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>'
    )
}

impl Lexer {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
            commands: Vec::new(),
            heredocs: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.pos
            .checked_add(1)
            .and_then(|next| self.chars.get(next))
            .copied()
    }

    fn advance(&mut self) {
        self.pos = self.pos.saturating_add(1);
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.advance();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        let matched = self.peek() == Some(c);
        if matched {
            self.advance();
        }
        matched
    }

    /// The source text from `start` to the cursor.
    fn raw_since(&self, start: usize) -> String {
        self.chars
            .get(start..self.pos)
            .into_iter()
            .flatten()
            .collect()
    }

    /// Reads commands until `close`, pushing each finished one.
    fn list(&mut self, close: Close, depth: usize) -> Result<(), ParseError> {
        if depth > MAX_NESTING {
            return Err(ParseError::TooDeep);
        }
        let mut command = SimpleCommand::default();
        loop {
            self.skip_blanks();
            let Some(c) = self.peek() else {
                self.finish(&mut command);
                return unclosed(close);
            };
            let closes = (c == ')' && matches!(close, Close::Paren(_)))
                || (c == '`' && close == Close::Backtick);
            if closes {
                self.advance();
                self.finish(&mut command);
                return Ok(());
            }
            self.step(&mut command, close, depth)?;
        }
    }

    /// Reads the token at the cursor into `command`.
    fn step(
        &mut self,
        command: &mut SimpleCommand,
        close: Close,
        depth: usize,
    ) -> Result<(), ParseError> {
        let Some(c) = self.peek() else {
            return Ok(());
        };
        match c {
            '#' => self.skip_comment(),
            '\n' => {
                self.advance();
                self.finish(command);
                self.heredoc_bodies(depth)?;
            }
            '&' if self.peek_next() == Some('>') => self.redirect(command, close, depth)?,
            // An unmatched `)` (a `case` pattern) only separates.
            ';' | '&' | '|' | ')' => {
                self.advance();
                self.finish(command);
            }
            '(' => {
                self.advance();
                self.finish(command);
                self.list(Close::Paren("("), depth.saturating_add(1))?;
            }
            '<' | '>' => self.redirect(command, close, depth)?,
            _ => {
                let word = self.word(close, depth)?;
                if word.is_fd() && matches!(self.peek(), Some('<' | '>')) {
                    self.redirect(command, close, depth)?;
                } else if word.is_present() {
                    command.words.push(word);
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self, command: &mut SimpleCommand) {
        if !command.words.is_empty() || !command.writes.is_empty() {
            self.commands.push(mem::take(command));
        }
    }

    fn skip_blanks(&mut self) {
        loop {
            match (self.peek(), self.peek_next()) {
                (Some(' ' | '\t'), _) => self.advance(),
                (Some('\\'), Some('\n')) => {
                    self.advance();
                    self.advance();
                }
                _ => return,
            }
        }
    }

    fn skip_comment(&mut self) {
        while self.peek().is_some_and(|c| c != '\n') {
            self.advance();
        }
    }

    /// Reads a redirection operator and its target.
    fn redirect(
        &mut self,
        command: &mut SimpleCommand,
        close: Close,
        depth: usize,
    ) -> Result<(), ParseError> {
        match self.operator() {
            Redirect::Process => self.list(Close::Paren("<("), depth.saturating_add(1)),
            Redirect::Heredoc { strip_tabs } => {
                let delimiter = self.target(close, depth)?;
                self.heredocs.push(Heredoc {
                    expands: !delimiter.quoted,
                    delimiter: delimiter.text,
                    strip_tabs,
                });
                Ok(())
            }
            Redirect::Write => {
                let target = self.target(close, depth)?;
                command.writes.push(target);
                Ok(())
            }
            Redirect::Duplicate => {
                let target = self.target(close, depth)?;
                if !(target.is_fd() || target.text == "-") {
                    command.writes.push(target);
                }
                Ok(())
            }
            Redirect::Read => self.target(close, depth).map(drop),
        }
    }

    /// Consumes a redirection operator (the cursor is on `<`, `>` or `&>`).
    fn operator(&mut self) -> Redirect {
        let first = self.bump();
        let second = self.peek();
        let (two_chars, redirect) = match (first, second) {
            // `&>` (and `&>>`), `<>`, `>>`, `>|`.
            (Some('&'), _) | (Some('<'), Some('>')) | (Some('>'), Some('>' | '|')) => {
                (true, Redirect::Write)
            }
            (Some('<'), Some('<')) => {
                self.advance();
                if self.eat('<') {
                    return Redirect::Read;
                }
                let strip_tabs = self.eat('-');
                return Redirect::Heredoc { strip_tabs };
            }
            (Some('<' | '>'), Some('(')) => (true, Redirect::Process),
            (Some('<'), Some('&')) => (true, Redirect::Read),
            (Some('>'), Some('&')) => (true, Redirect::Duplicate),
            (Some('<'), _) => (false, Redirect::Read),
            _ => (false, Redirect::Write),
        };
        if two_chars {
            self.advance();
        }
        if first == Some('&') && self.peek() == Some('>') {
            // The second `>` of `&>>`.
            self.advance();
        }
        redirect
    }

    /// The word after a redirection operator.
    fn target(&mut self, close: Close, depth: usize) -> Result<Word, ParseError> {
        self.skip_blanks();
        if self.peek().is_none_or(is_meta) {
            return Err(ParseError::MissingRedirectTarget);
        }
        let word = self.word(close, depth)?;
        if word.is_present() {
            Ok(word)
        } else {
            Err(ParseError::MissingRedirectTarget)
        }
    }
}

/// The result of reaching the end of the input while reading up to `close`.
const fn unclosed(close: Close) -> Result<(), ParseError> {
    match close {
        Close::End => Ok(()),
        Close::Paren(what) => Err(ParseError::Unclosed { what }),
        Close::Backtick => Err(ParseError::Unclosed { what: "`" }),
    }
}

#[cfg(test)]
mod tests;
