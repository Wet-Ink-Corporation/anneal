//! Words: quote removal, escapes, and the expansions that make a word dynamic.

use super::{Close, Lexer, MAX_NESTING, ParseError, Quoting, Shape, Word, is_meta};

impl Lexer {
    /// Reads one word, removing quotes and escapes.
    pub(super) fn word(&mut self, close: Close, depth: usize) -> Result<Word, ParseError> {
        let mut word = Word::new();
        while let Some(c) = self.peek() {
            if is_meta(c) || (c == '`' && close == Close::Backtick) {
                break;
            }
            self.advance();
            self.word_char(c, &mut word, depth)?;
        }
        // A lone `{`, `[` or `[[` is a keyword or the test command, not a pattern.
        if !word.quoted && matches!(word.text.as_str(), "{" | "[" | "[[") {
            word.shape = Shape::Literal;
        }
        Ok(word)
    }

    /// Adds the already consumed unquoted character `c` (and what it opens) to `word`.
    fn word_char(&mut self, c: char, word: &mut Word, depth: usize) -> Result<(), ParseError> {
        match c {
            '\\' => match self.bump() {
                Some('\n') => {}
                Some(escaped) => {
                    word.quoted = true;
                    word.text.push(escaped);
                }
                None => word.text.push('\\'),
            },
            '\'' => self.single_quoted(word)?,
            '"' => {
                word.quoted = true;
                self.quoted_body(word, Some('"'), depth)?;
            }
            '$' => self.dollar(word, Quoting::Bare, depth)?,
            '`' => self.backticks(word, depth)?,
            '*' | '?' | '[' | '{' => {
                word.mark(Shape::Pattern);
                word.text.push(c);
            }
            '~' if word.text.is_empty() && !word.quoted => {
                word.mark(Shape::Pattern);
                word.text.push(c);
            }
            _ => word.text.push(c),
        }
        Ok(())
    }

    fn single_quoted(&mut self, word: &mut Word) -> Result<(), ParseError> {
        word.quoted = true;
        loop {
            match self.bump() {
                None => return Err(ParseError::Unclosed { what: "'" }),
                Some('\'') => return Ok(()),
                Some(c) => word.text.push(c),
            }
        }
    }

    /// The inside of double quotes (up to `terminator`), or a heredoc body (no terminator):
    /// `\` escapes only `$`, `` ` ``, `"`, `\` and a newline, and substitutions run.
    pub(super) fn quoted_body(
        &mut self,
        word: &mut Word,
        terminator: Option<char>,
        depth: usize,
    ) -> Result<(), ParseError> {
        loop {
            let Some(c) = self.bump() else {
                return match terminator {
                    Some(_) => Err(ParseError::Unclosed { what: "\"" }),
                    None => Ok(()),
                };
            };
            match c {
                '"' if terminator == Some('"') => return Ok(()),
                '\\' => match self.peek() {
                    Some(escaped @ ('$' | '`' | '"' | '\\')) => {
                        self.advance();
                        word.text.push(escaped);
                    }
                    Some('\n') => self.advance(),
                    _ => word.text.push('\\'),
                },
                '$' => self.dollar(word, Quoting::Double, depth)?,
                '`' => self.backticks(word, depth)?,
                _ => word.text.push(c),
            }
        }
    }

    /// After a `$`: a substitution, a variable, ANSI-C or locale quoting, or a plain `$`.
    fn dollar(
        &mut self,
        word: &mut Word,
        quoting: Quoting,
        depth: usize,
    ) -> Result<(), ParseError> {
        let start = self.pos.saturating_sub(1);
        let inner = depth.saturating_add(1);
        match self.peek() {
            Some('(') => {
                self.advance();
                self.list(Close::Paren("$("), inner)?;
            }
            Some('{') => {
                self.advance();
                self.braced(inner)?;
            }
            Some('\'') if quoting == Quoting::Bare => {
                self.advance();
                return self.ansi_c(word);
            }
            Some('"') if quoting == Quoting::Bare => {
                self.advance();
                word.quoted = true;
                return self.quoted_body(word, Some('"'), depth);
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    self.advance();
                }
            }
            Some(c) if c.is_ascii_digit() || "@*#?-$!".contains(c) => self.advance(),
            _ => {
                word.text.push('$');
                return Ok(());
            }
        }
        word.mark(Shape::Dynamic);
        word.text.push_str(&self.raw_since(start));
        Ok(())
    }

    /// The inside of `${ ... }`, which may hold quotes and further substitutions.
    fn braced(&mut self, depth: usize) -> Result<(), ParseError> {
        if depth > MAX_NESTING {
            return Err(ParseError::TooDeep);
        }
        let mut scratch = Word::new();
        loop {
            match self.bump() {
                None => return Err(ParseError::Unclosed { what: "${" }),
                Some('}') => return Ok(()),
                Some('\\') => self.advance(),
                Some('\'') => self.single_quoted(&mut scratch)?,
                Some('"') => self.quoted_body(&mut scratch, Some('"'), depth)?,
                Some('$') => self.dollar(&mut scratch, Quoting::Bare, depth)?,
                Some('`') => self.backticks(&mut scratch, depth)?,
                Some(_) => {}
            }
        }
    }

    fn backticks(&mut self, word: &mut Word, depth: usize) -> Result<(), ParseError> {
        let start = self.pos.saturating_sub(1);
        self.list(Close::Backtick, depth.saturating_add(1))?;
        word.mark(Shape::Dynamic);
        word.text.push_str(&self.raw_since(start));
        Ok(())
    }

    /// The inside of `$'...'`, with its escapes decoded.
    fn ansi_c(&mut self, word: &mut Word) -> Result<(), ParseError> {
        word.quoted = true;
        loop {
            match self.bump() {
                None => return Err(ParseError::Unclosed { what: "$'" }),
                Some('\'') => return Ok(()),
                Some('\\') => self.ansi_escape(word),
                Some(c) => word.text.push(c),
            }
        }
    }

    /// One `$'...'` escape after its backslash.
    fn ansi_escape(&mut self, word: &mut Word) {
        let Some(c) = self.bump() else {
            word.text.push('\\');
            return;
        };
        let decoded = match c {
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            'a' => Some('\u{7}'),
            'b' => Some('\u{8}'),
            'e' | 'E' => Some('\u{1b}'),
            'f' => Some('\u{c}'),
            'v' => Some('\u{b}'),
            'x' => self.code_point(16, 2, 0),
            'u' => self.code_point(16, 4, 0),
            'U' => self.code_point(16, 8, 0),
            '0'..='7' => c.to_digit(8).and_then(|first| self.code_point(8, 2, first)),
            other => Some(other),
        };
        if let Some(decoded) = decoded {
            word.text.push(decoded);
        } else {
            word.text.push('\\');
            word.text.push(c);
        }
    }

    /// Reads up to `max_digits` digits in `radix` onto `value`; `None` when a hex escape has no
    /// digit or the value is not a character.
    fn code_point(&mut self, radix: u32, max_digits: usize, value: u32) -> Option<char> {
        let mut value = value;
        let mut digits = 0;
        while digits < max_digits {
            let Some(digit) = self.peek().and_then(|c| c.to_digit(radix)) else {
                break;
            };
            value = value.saturating_mul(radix).saturating_add(digit);
            self.advance();
            digits += 1;
        }
        if radix == 16 && digits == 0 {
            return None;
        }
        char::from_u32(value)
    }
}
