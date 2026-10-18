//! Words: quote removal, escapes, and the expansions that make a word dynamic.

use std::ops::Range;

use super::extglob::{self, Place};
use super::{Close, Lexer, MAX_NESTING, ParseError, Quoting, Reading, is_meta};

/// A shell word after quote removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    /// The word's text with quotes and escapes removed; an expansion keeps its source text.
    pub(crate) text: String,
    /// How far the text is from what the shell passes on.
    pub(crate) shape: Shape,
    /// Whether any part of the word was quoted or escaped.
    pub(crate) quoted: bool,
    /// Whether the word holds an unquoted `*`, `?`, `[`, `{` or a leading `~`, whatever its
    /// shape: `"$b"{-rf,/}` is [`Shape::Dynamic`] and still brace-expands.
    pub(crate) globs: Globs,
    /// Each expansion's source text in `text` (`$x`, `${x}`, `$( )`, backticks, `<( )`).
    pub(crate) expansions: Vec<Expansion>,
    /// The byte ranges of `text` that brace expansion does not read, in order: each quoted or
    /// escaped part (empty for `''`), and a known variable value the denylist put in, which
    /// the shell expands only after braces.
    pub(crate) quotes: Vec<Range<usize>>,
    /// Whether it was read inside an arithmetic expression: in a command of one (see
    /// [`super::SimpleCommand::arithmetic`]), or from an unquoted `$[` up to its `]`, which the
    /// lexer splits into words as it always has.
    pub(crate) arithmetic: bool,
}

/// Where an expansion's source text is in its word's text, and whether it was in double
/// quotes (so its value is not split into words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expansion {
    /// The byte range of the source text.
    pub(crate) range: Range<usize>,
    /// Whether it was inside double quotes (or a heredoc body).
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

/// Whether a word holds an unquoted pattern character (`*`, `?`, `[`, `{` or a leading `~`),
/// which [`Shape`] does not tell once a word is also dynamic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Globs {
    /// None unquoted.
    Absent,
    /// At least one unquoted.
    Unquoted,
}

impl Globs {
    /// [`Globs::Unquoted`] when `found`.
    pub(crate) const fn found(found: bool) -> Self {
        if found { Self::Unquoted } else { Self::Absent }
    }
}

impl Word {
    pub(super) const fn new() -> Self {
        Self {
            text: String::new(),
            shape: Shape::Literal,
            quoted: false,
            globs: Globs::Absent,
            expansions: Vec::new(),
            quotes: Vec::new(),
            arithmetic: false,
        }
    }

    /// A quoted word whose text is exactly `text`.
    pub(super) fn quoted(text: String) -> Self {
        Self {
            quotes: Self::all_quoted(&text),
            text,
            shape: Shape::Literal,
            quoted: true,
            globs: Globs::Absent,
            expansions: Vec::new(),
            arithmetic: false,
        }
    }

    /// [`Word::quotes`] for a word whose text is all quoted: one range over `text`.
    pub(crate) fn all_quoted(text: &str) -> Vec<Range<usize>> {
        std::iter::once(0..text.len()).collect()
    }

    /// Runs `read` on the word and records the text it adds as quoted.
    fn quoting<T>(&mut self, read: impl FnOnce(&mut Self) -> T) -> T {
        let start = self.text.len();
        let read = read(self);
        self.quotes.push(start..self.text.len());
        read
    }

    fn mark(&mut self, shape: Shape) {
        self.shape = self.shape.max(shape);
    }

    /// Adds the unquoted pattern character `c`, which sets [`Word::globs`] whatever the shape.
    pub(super) fn push_pattern(&mut self, c: char) {
        self.mark(Shape::Pattern);
        self.globs = Globs::Unquoted;
        self.text.push(c);
    }

    /// Whether the word holds an unquoted pattern character, whatever its shape.
    pub(crate) fn has_pattern(&self) -> bool {
        self.globs == Globs::Unquoted
    }

    /// The word's text outside its expansions' source text, in order; `None` when an
    /// expansion's range is not in the text, which the lexer never makes.
    pub(crate) fn outside_expansions(&self) -> Option<Vec<&str>> {
        let mut parts = Vec::with_capacity(self.expansions.len().saturating_add(1));
        let mut at = 0;
        for expansion in &self.expansions {
            parts.push(self.text.get(at..expansion.range.start)?);
            at = expansion.range.end;
        }
        parts.push(self.text.get(at..)?);
        Some(parts)
    }

    /// [`Word::quotes`] moved to where they are once each expansion source range in `replaced`
    /// (in order, each paired with the length of the text that takes its place) is replaced. A
    /// quote that holds a replaced range grows or shrinks with it. One pass over both (H-13).
    pub(crate) fn quotes_replacing<'r>(
        &self,
        replaced: impl IntoIterator<Item = (&'r Range<usize>, usize)>,
    ) -> Vec<Range<usize>> {
        let mut ahead = replaced.into_iter().peekable();
        let (mut grown, mut shrunk) = (0_usize, 0_usize);
        // Offsets come in ascending order, and none is strictly inside a replaced range.
        let mut moved = |at: usize| {
            while let Some((range, len)) = ahead.next_if(|(range, _)| range.end <= at) {
                grown = grown.saturating_add(len);
                shrunk = shrunk.saturating_add(range.len());
            }
            at.saturating_add(grown).saturating_sub(shrunk)
        };
        self.quotes
            .iter()
            .map(|quote| {
                let start = moved(quote.start);
                start..moved(quote.end)
            })
            .collect()
    }

    /// Whether the text is exactly what runs.
    pub(crate) fn is_literal(&self) -> bool {
        self.shape == Shape::Literal
    }

    /// Whether the word exists at all: `''` does, an unquoted nothing does not.
    pub(super) fn is_present(&self) -> bool {
        !self.text.is_empty() || self.quoted || !self.is_literal()
    }

    /// An unquoted run of digits, which before `<` or `>` names a file descriptor.
    pub(super) fn is_fd(&self) -> bool {
        !self.quoted
            && self.is_literal()
            && !self.text.is_empty()
            && self.text.chars().all(|c| c.is_ascii_digit())
    }

    /// Whether the word is one process substitution, `<( )` or `>( )`: a pipe's path.
    pub(crate) fn is_process_substitution(&self) -> bool {
        matches!(self.expansions.as_slice(), [only] if only.range == (0..self.text.len()))
            && (self.text.starts_with("<(") || self.text.starts_with(">("))
    }

    /// An unquoted literal word, which may be a reserved word.
    pub(super) fn is_bare(&self, reserved: &[&str]) -> bool {
        !self.quoted && self.is_literal() && reserved.contains(&self.text.as_str())
    }
}

impl Lexer {
    /// Reads one word, removing quotes and escapes. `place` says whether a `!(` that starts
    /// it is an extended glob or the `!` keyword before a subshell.
    pub(super) fn word(
        &mut self,
        close: Close,
        depth: usize,
        place: Place,
    ) -> Result<Word, ParseError> {
        let mut word = Word::new();
        word.arithmetic = self.in_arithmetic();
        while let Some(c) = self.peek() {
            if c == '(' && extglob::opens(&word, place) {
                self.advance();
                self.extglob(&mut word, depth)?;
                continue;
            }
            if is_meta(c) || (c == '`' && close == Close::Backtick) {
                break;
            }
            self.advance();
            self.word_char(c, &mut word, depth)?;
        }
        // A lone `{`, `[` or `[[` is a keyword or the test command, not a pattern.
        if !word.quoted && matches!(word.text.as_str(), "{" | "[" | "[[") {
            word.shape = Shape::Literal;
            word.globs = Globs::Absent;
        }
        Ok(word)
    }

    /// Adds the already consumed unquoted character `c` (and what it opens) to `word`.
    pub(super) fn word_char(
        &mut self,
        c: char,
        word: &mut Word,
        depth: usize,
    ) -> Result<(), ParseError> {
        match c {
            '\\' => match self.bump() {
                Some('\n') => {}
                Some(escaped) => {
                    word.quoted = true;
                    word.quoting(|word| word.text.push(escaped));
                }
                None => word.text.push('\\'),
            },
            '\'' => word.quoting(|word| self.single_quoted(word))?,
            '"' => {
                word.quoted = true;
                word.quoting(|word| self.quoted_body(word, Some('"'), depth))?;
            }
            '$' => self.dollar(word, Quoting::Bare, depth)?,
            '`' => self.backticks(word, Quoting::Bare, depth)?,
            '*' | '?' | '[' | '{' => {
                // A `[` inside `$[ ]` (an array subscript) is closed by a `]` of its own.
                if c == '[' && self.brackets > 0 {
                    self.brackets = self.brackets.saturating_add(1);
                }
                word.push_pattern(c);
            }
            ']' => {
                self.brackets = self.brackets.saturating_sub(1);
                word.text.push(c);
            }
            '~' if word.text.is_empty() && !word.quoted => word.push_pattern(c),
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
                '`' => self.backticks(word, Quoting::Double, depth)?,
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
                // `$((` is arithmetic; its commands are listed as those of `$( (` would be.
                let reading = if self.peek() == Some('(') {
                    Reading::Arithmetic
                } else {
                    Reading::Commands
                };
                self.nested(Close::Paren("$("), inner, reading)?;
            }
            Some('{') => {
                self.advance();
                self.braced(inner)?;
            }
            Some('[') if quoting == Quoting::Bare => {
                // `$[ ]`, the old arithmetic form, still read as a `$` and a pattern's `[`,
                // its words split as before: only marked, up to the `]` that closes it.
                self.advance();
                self.brackets = self.brackets.saturating_add(1);
                word.arithmetic = true;
                word.mark(Shape::Pattern);
                word.globs = Globs::Unquoted;
                word.text.push_str("$[");
                return Ok(());
            }
            Some('\'') if quoting == Quoting::Bare => {
                self.advance();
                return word.quoting(|word| self.ansi_c(word));
            }
            Some('"') if quoting == Quoting::Bare => {
                self.advance();
                word.quoted = true;
                return word.quoting(|word| self.quoted_body(word, Some('"'), depth));
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
        self.push_expansion(word, start, quoting);
        Ok(())
    }

    /// Adds the source text from `start` to `word` as an expansion.
    pub(super) fn push_expansion(&self, word: &mut Word, start: usize, quoting: Quoting) {
        word.mark(Shape::Dynamic);
        let at = word.text.len();
        word.text.push_str(&self.raw_since(start));
        word.expansions.push(Expansion {
            range: at..word.text.len(),
            quoted: quoting == Quoting::Double,
        });
    }

    /// The inside of `${ ... }`, which may hold quotes and further substitutions.
    fn braced(&mut self, depth: usize) -> Result<(), ParseError> {
        if depth > MAX_NESTING {
            return Err(ParseError::TooDeep);
        }
        let mut scratch = Word::new();
        // A `$[` inside is closed by a `]` this loop does not count.
        let brackets = self.brackets;
        loop {
            match self.bump() {
                None => return Err(ParseError::Unclosed { what: "${" }),
                Some('}') => {
                    self.brackets = brackets;
                    return Ok(());
                }
                Some('\\') => self.advance(),
                Some('\'') => self.single_quoted(&mut scratch)?,
                Some('"') => self.quoted_body(&mut scratch, Some('"'), depth)?,
                Some('$') => self.dollar(&mut scratch, Quoting::Bare, depth)?,
                Some('`') => self.backticks(&mut scratch, Quoting::Bare, depth)?,
                Some(_) => {}
            }
        }
    }

    fn backticks(
        &mut self,
        word: &mut Word,
        quoting: Quoting,
        depth: usize,
    ) -> Result<(), ParseError> {
        let start = self.pos.saturating_sub(1);
        self.nested(Close::Backtick, depth.saturating_add(1), Reading::Commands)?;
        self.push_expansion(word, start, quoting);
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
