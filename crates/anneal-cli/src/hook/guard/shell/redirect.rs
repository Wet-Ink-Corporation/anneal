//! Redirections: the files a command writes, where its standard input comes from, and the
//! heredocs whose bodies follow the line.

use super::{
    Close, Level, Lexer, ParseError, Place, Quoting, Reading, SimpleCommand, Stdin, Word, Writer,
};

/// What a redirection operator does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redirect {
    /// `>`, `>>`, `>|`, `&>`, `&>>`, `<>`: the target is written.
    Write,
    /// `>&`: a file when the target is not a descriptor.
    Duplicate,
    /// `<`, `<&`: the target is read.
    Read,
    /// `<<<`: the target is the input.
    HereString,
    /// `<<` or `<<-`.
    Heredoc { strip_tabs: bool },
    /// `<(` or `>(`.
    Process,
}

/// A heredoc whose body starts after the next newline.
#[derive(Debug)]
pub(super) struct Heredoc {
    pub(super) delimiter: String,
    /// An unquoted delimiter: the body's substitutions run.
    pub(super) expands: bool,
    pub(super) strip_tabs: bool,
    /// Which heredoc of the input it is, which names its body.
    pub(super) number: usize,
}

impl Lexer {
    /// Reads a redirection operator and its target into `command`; `on_stdin` when the
    /// operator has no descriptor or descriptor 0, so an input redirection is its stdin.
    pub(super) fn redirect(
        &mut self,
        command: &mut SimpleCommand,
        level: Level,
        on_stdin: bool,
    ) -> Result<(), ParseError> {
        match self.operator() {
            Redirect::Process => {
                let word = self.process(self.pos.saturating_sub(2), level.depth)?;
                command.words.push(word);
                Ok(())
            }
            Redirect::Heredoc { strip_tabs } => {
                let delimiter = self.target(level)?;
                let number = self.heredoc_count;
                self.heredoc_count = number.saturating_add(1);
                self.heredocs.push(Heredoc {
                    expands: !delimiter.quoted,
                    delimiter: delimiter.text,
                    strip_tabs,
                    number,
                });
                if on_stdin {
                    command.set_stdin(Stdin::Inherited);
                    command.heredoc = Some(number);
                }
                Ok(())
            }
            Redirect::Write => {
                let target = self.target(level)?;
                command.writes.push(target);
                Ok(())
            }
            Redirect::Duplicate => {
                let target = self.target(level)?;
                if !(target.is_fd() || target.text == "-") {
                    command.writes.push(target);
                }
                Ok(())
            }
            Redirect::Read => {
                let source = self.target(level)?;
                if on_stdin {
                    command.set_stdin(if source.is_process_substitution() {
                        Stdin::Pipe(None)
                    } else {
                        Stdin::File
                    });
                }
                Ok(())
            }
            Redirect::HereString => {
                let text = self.target(level)?;
                if on_stdin {
                    command.set_stdin(Stdin::Text(text));
                }
                Ok(())
            }
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
                    return Redirect::HereString;
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
    fn target(&mut self, level: Level) -> Result<Word, ParseError> {
        self.skip_blanks();
        if matches!(self.peek(), Some('<' | '>')) && self.peek_next() == Some('(') {
            // `done < <(ls)`: the target is a process substitution.
            let start = self.pos;
            self.advance();
            self.advance();
            return self.process(start, level.depth);
        }
        if self.peek().is_none_or(super::is_meta) {
            return Err(ParseError::MissingRedirectTarget);
        }
        let word = self.word(level.close, level.depth, Place::Argument)?;
        if word.is_present() {
            Ok(word)
        } else {
            Err(ParseError::MissingRedirectTarget)
        }
    }

    /// Reads the commands of the process substitution whose `<(` or `>(` starts at `start`
    /// (the cursor is past it), returning it as a dynamic word. Inside `>( )` standard input
    /// is a pipe from the outer command.
    fn process(&mut self, start: usize, depth: usize) -> Result<Word, ParseError> {
        let mut word = Word::new();
        self.process_into(&mut word, start, depth)?;
        Ok(word)
    }

    /// Reads the commands of the process substitution whose `<(` or `>(` starts at `start`
    /// and adds it to `word` as an expansion (inside an extended glob's group, `@(<(ls))`).
    pub(super) fn process_into(
        &mut self,
        word: &mut Word,
        start: usize,
        depth: usize,
    ) -> Result<(), ParseError> {
        let outer = self.pipe;
        if self.chars.get(start) == Some(&'>') {
            self.pipe = Some(Writer(None));
        }
        self.nested(
            Close::Paren("<("),
            depth.saturating_add(1),
            Reading::Commands,
        )?;
        self.pipe = outer;
        self.push_expansion(word, start, Quoting::Bare);
        Ok(())
    }
}

impl SimpleCommand {
    /// Takes standard input from `stdin`, replacing an earlier redirection or heredoc.
    fn set_stdin(&mut self, stdin: Stdin) {
        self.stdin = stdin;
        self.heredoc = None;
    }
}
