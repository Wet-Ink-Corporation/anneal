//! A POSIX-ish shell lexer: enough of `sh` to see every simple command a Bash tool call runs.
//!
//! [`parse`] removes quotes (single, double, ANSI-C `$'...'`) and backslash escapes, splits at
//! `&&`, `||`, `;`, `|`, `&` and newlines, and flattens `$( )`, backticks, `( )` subshells,
//! `<( )` process substitutions and the substitutions in unquoted heredoc bodies into the same
//! list, so a command cannot hide inside another. A `case` pattern's `)` does not close a
//! substitution. Heredoc bodies are data, kept as the standard input of the command reading
//! them. Expansions are not performed: a word that has one is marked [`Shape::Dynamic`], one
//! with an unquoted glob, brace or leading `~` [`Shape::Pattern`].
//!
//! Commands are read here; words (quotes, escapes, `$`) in [`word`], redirections in
//! [`redirect`], heredoc bodies in [`heredoc`].

mod heredoc;
mod redirect;
mod word;

use std::mem;

use redirect::Heredoc;
pub(crate) use word::{Expansion, Shape, Word};

/// How deep `$( )`, backticks, `( )` and `${ }` may nest before a command is refused (H-13).
pub(crate) const MAX_NESTING: usize = 32;

/// Words that may come before a reserved word such as `esac` in command position.
const RESERVED: [&str; 10] = [
    "!", "{", "if", "then", "else", "elif", "do", "while", "until", "time",
];

/// A simple command: its words, the files its redirections write, and its standard input.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SimpleCommand {
    /// The words, program first.
    pub(crate) words: Vec<Word>,
    /// The targets of `>`, `>>`, `>|`, `&>`, `<>` and `>&file`.
    pub(crate) writes: Vec<Word>,
    /// Where its standard input comes from.
    pub(crate) stdin: Stdin,
    /// Whether a `(` follows its words, as in a function definition (`name()`, `function
    /// name()`). One after keywords alone (`if (`), after `case ... in` or after an array
    /// assignment (`a=(`) does not count.
    pub(crate) defines_function: bool,
    /// Whether it was read inside an arithmetic expression (`$(( ))`, `(( ))`, or a `( )`
    /// group inside one or inside `$[ ]`): its words are operands and operators, not a program
    /// and its arguments. It is listed as any other command, so the denylist reads it as before.
    pub(crate) arithmetic: bool,
    /// The heredoc it reads, by number, until the body is read.
    heredoc: Option<usize>,
}

/// Where a simple command's standard input comes from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Stdin {
    /// The standard input of the whole command.
    #[default]
    Inherited,
    /// A file (`<`, `<&`).
    File,
    /// A here-string or heredoc body.
    Text(Word),
    /// A pipe: the index (in [`parse`]'s list) of the command writing it, when that is the
    /// command just before the `|`.
    Pipe(Option<usize>),
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
    Ok(lexer.into_commands())
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

/// The command list being read: what ends it, how deep it is, and the pipe and open `case`
/// count it started with.
#[derive(Debug, Clone, Copy)]
struct Level {
    close: Close,
    depth: usize,
    pipe: Option<Writer>,
    cases: usize,
}

/// The command writing the pipe the next commands read (its index, when known).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Writer(Option<usize>);

/// What the list being read holds: commands, or an arithmetic expression (`$(( ))`,
/// `(( ))`) whose words only look like commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Reading {
    /// Commands: the top level, `$( )`, backticks, `<( )` and plain subshells.
    #[default]
    Commands,
    /// An arithmetic expression, and any `( )` group inside it.
    Arithmetic,
}

/// Whether `$` inside double quotes, where `$'` and `$"` are not quoting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quoting {
    Bare,
    Double,
}

/// The lexer's state: the input, the cursor, the commands read so far, the heredocs whose
/// bodies start at the next newline, the bodies read, the pipe the next command reads, the
/// `case` statements open, what the list being read holds and the unquoted `$[` it has open.
#[derive(Debug)]
struct Lexer {
    chars: Vec<char>,
    pos: usize,
    commands: Vec<SimpleCommand>,
    heredocs: Vec<Heredoc>,
    /// Heredoc bodies by number; `None` until read.
    bodies: Vec<Option<Word>>,
    /// How many heredocs have been opened, which numbers the next one.
    heredoc_count: usize,
    pipe: Option<Writer>,
    cases: usize,
    reading: Reading,
    /// Unquoted `$[` (and `[` inside one) not yet closed by `]` in the list being read.
    brackets: usize,
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
            bodies: Vec::new(),
            heredoc_count: 0,
            pipe: None,
            cases: 0,
            reading: Reading::Commands,
            brackets: 0,
        }
    }

    /// Whether what is read now is part of an arithmetic expression: in an arithmetic list, or
    /// after an unquoted `$[` not yet closed.
    const fn in_arithmetic(&self) -> bool {
        matches!(self.reading, Reading::Arithmetic) || self.brackets > 0
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

    /// Reads the list a substitution, subshell or arithmetic expression opens, up to `close`,
    /// as `reading` and with no `$[` open, then restores both.
    fn nested(&mut self, close: Close, depth: usize, reading: Reading) -> Result<(), ParseError> {
        let outer = (self.reading, self.brackets);
        (self.reading, self.brackets) = (reading, 0);
        let read = self.list(close, depth);
        (self.reading, self.brackets) = outer;
        read
    }

    /// Reads commands until `close`, pushing each finished one.
    fn list(&mut self, close: Close, depth: usize) -> Result<(), ParseError> {
        if depth > MAX_NESTING {
            return Err(ParseError::TooDeep);
        }
        let level = Level {
            close,
            depth,
            pipe: self.pipe,
            cases: self.cases,
        };
        let mut command = SimpleCommand::default();
        loop {
            self.skip_blanks();
            let Some(c) = self.peek() else {
                self.finish(&mut command);
                return unclosed(close);
            };
            // A `)` while a `case` opened in this list is open ends a pattern, not the list.
            let closes =
                (c == ')' && matches!(close, Close::Paren(_)) && self.cases <= level.cases)
                    || (c == '`' && close == Close::Backtick);
            if closes {
                self.advance();
                self.finish(&mut command);
                self.pipe = level.pipe;
                self.cases = level.cases;
                return Ok(());
            }
            self.step(&mut command, level)?;
        }
    }

    /// Reads the token at the cursor into `command`.
    fn step(&mut self, command: &mut SimpleCommand, level: Level) -> Result<(), ParseError> {
        let Some(c) = self.peek() else {
            return Ok(());
        };
        match c {
            '#' => self.skip_comment(),
            '\n' => {
                self.advance();
                self.separate(command, level);
                self.heredoc_bodies(level.depth)?;
            }
            '&' if self.peek_next() == Some('>') => self.redirect(command, level, true)?,
            '|' => {
                self.advance();
                if self.eat('|') {
                    self.separate(command, level);
                } else {
                    // `|&` pipes stderr too.
                    self.eat('&');
                    let writer = self.finish(command);
                    self.pipe = Some(Writer(writer));
                }
            }
            // An unmatched `)` (a `case` pattern) only separates.
            ';' | '&' | ')' => {
                self.advance();
                self.separate(command, level);
            }
            '(' => {
                self.advance();
                command.defines_function = opens_definition(&command.words);
                self.finish(command);
                // `((` opens an arithmetic command (bash reads `((a) )` as two subshells, so
                // this may take a subshell for arithmetic: it then only marks more, and the
                // commands listed are the same). A group inside arithmetic or `$[` is
                // arithmetic too.
                let reading = if self.peek() == Some('(') || self.in_arithmetic() {
                    Reading::Arithmetic
                } else {
                    Reading::Commands
                };
                self.nested(Close::Paren("("), level.depth.saturating_add(1), reading)?;
            }
            '<' | '>' => self.redirect(command, level, true)?,
            _ => {
                let word = self.word(level.close, level.depth)?;
                if word.is_fd() && matches!(self.peek(), Some('<' | '>')) {
                    self.redirect(command, level, word.text == "0")?;
                } else if word.is_present() {
                    self.count_case(command, &word, level);
                    command.words.push(word);
                }
            }
        }
        Ok(())
    }

    /// Counts an unquoted `case` wherever it is, and an `esac` in command position.
    ///
    /// Bash also opens a `case` after `time -p`, `function NAME` or `coproc NAME`, so any
    /// bare `case` counts: a stray one (`$(echo case)`) leaves its substitution unclosed, which
    /// denies, where missing one would let a pattern's `)` close the substitution early.
    fn count_case(&mut self, command: &SimpleCommand, word: &Word, level: Level) {
        if word.is_bare(&["case"]) {
            self.cases = self.cases.saturating_add(1);
        } else if word.is_bare(&["esac"])
            && self.cases > level.cases
            && command.words.iter().all(|before| before.is_bare(&RESERVED))
        {
            self.cases = self.cases.saturating_sub(1);
        }
    }

    /// Ends `command` at a separator: the next command no longer reads the pipe, unless the
    /// whole list does.
    fn separate(&mut self, command: &mut SimpleCommand, level: Level) {
        self.finish(command);
        self.pipe = level.pipe;
    }

    /// Pushes `command` if it has anything, returning its index.
    fn finish(&mut self, command: &mut SimpleCommand) -> Option<usize> {
        if command.words.is_empty() && command.writes.is_empty() {
            return None;
        }
        let reads_pipe = command.stdin == Stdin::Inherited && command.heredoc.is_none();
        if let Some(Writer(writer)) = self.pipe.filter(|_| reads_pipe) {
            command.stdin = Stdin::Pipe(writer);
        }
        // A nested list restores `reading` before it returns, so this is the list's own.
        command.arithmetic = self.reading == Reading::Arithmetic;
        let index = self.commands.len();
        self.commands.push(mem::take(command));
        Some(index)
    }

    /// Appends `commands`, lexed apart (a heredoc body), shifting their pipe writers.
    fn adopt(&mut self, commands: Vec<SimpleCommand>) {
        let offset = self.commands.len();
        self.commands
            .extend(commands.into_iter().map(|mut command| {
                if let Stdin::Pipe(Some(writer)) = &mut command.stdin {
                    *writer = writer.saturating_add(offset);
                }
                command
            }));
    }

    /// The commands, each reading the heredoc body it named (empty if never read).
    fn into_commands(mut self) -> Vec<SimpleCommand> {
        for command in &mut self.commands {
            if let Some(number) = command.heredoc.take() {
                let body = self.bodies.get_mut(number).and_then(Option::take);
                command.stdin = Stdin::Text(body.unwrap_or_else(|| Word::quoted(String::new())));
            }
        }
        self.commands
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
}

/// Whether a `(` after `words` (the words of the command it ends) defines a function: some
/// word comes before it that is not a keyword, the last is not `in` (a `case` pattern) and
/// does not end with `=` (an array assignment). It may over-count (`[[ ( -f x ) ]]`), which
/// only makes the merge gate's probe more careful.
fn opens_definition(words: &[Word]) -> bool {
    let after_keywords = words.iter().all(|word| word.is_bare(&RESERVED));
    let pattern_or_array = words
        .last()
        .is_some_and(|word| word.is_bare(&["in"]) || word.text.ends_with('='));
    !after_keywords && !pattern_or_array
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
