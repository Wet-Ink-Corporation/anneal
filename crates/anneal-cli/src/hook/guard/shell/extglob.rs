//! Extended globs (`shopt -s extglob`): `@(a|b)`, `+( )`, `!( )`, `?( )` and `*( )` are one
//! pattern word, not a word followed by a subshell (#128).

use super::{Lexer, ParseError, Word};

/// Where a word is read: what an opening `!(` means there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// Where a program or keyword may stand: `!(x)` is the `!` keyword and a subshell, as bash
    /// reads it without `extglob`.
    Command,
    /// The word after `function`, a function's name: `!(x)` is an extended glob, and an empty
    /// `()` may be the definition's.
    Name,
    /// An argument or a redirection's target: `!(x)` is an extended glob.
    Argument,
}

impl Place {
    /// Whether a function's name may stand here, so a `()` after it may define the function.
    fn names_function(self) -> bool {
        matches!(self, Self::Command | Self::Name)
    }
}

/// The characters that open an extended glob when an unquoted `(` follows them.
pub(crate) const OPENERS: [char; 5] = ['@', '+', '!', '?', '*'];

/// Whether a `(` after `word` opens an extended glob's group: the word ends with an unquoted
/// [`OPENERS`] character, and that character is not a `!` standing alone in command position.
/// A special parameter's name counts (`$@(`, `$?(`): bash appends a `$` with no `(`, `{` or
/// quote after it as text and reads the next character as any other.
fn opens(word: &Word, place: Place) -> bool {
    let len = word.text.len();
    let unquoted_end = word.quotes.last().is_none_or(|quote| quote.end < len);
    let bang_keyword = place == Place::Command && word.text == "!";
    unquoted_end
        && !bang_keyword
        && word
            .text
            .chars()
            .next_back()
            .is_some_and(|c| OPENERS.contains(&c))
}

impl Lexer {
    /// Whether the `(` at the cursor opens an extended glob's group after `word`.
    ///
    /// # Errors
    /// [`ParseError::ExtglobName`] for an empty `()` (blanks allowed: `f?( )`) where a
    /// function's name may stand: bash without `extglob` reads a definition there, and with it
    /// one pattern word whose later words are its arguments (`/bin/echo@() { hi }` runs
    /// `echo`), so neither reading can be trusted. Elsewhere `@()` is an empty group.
    pub(super) fn opens_extglob(&self, word: &Word, place: Place) -> Result<bool, ParseError> {
        if self.peek() != Some('(') || !opens(word, place) {
            return Ok(false);
        }
        if place.names_function() && self.empty_parens() {
            return Err(ParseError::ExtglobName);
        }
        Ok(true)
    }

    /// Whether the `(` at the cursor is followed, past blanks, newlines and line continuations,
    /// by `)`.
    fn empty_parens(&self) -> bool {
        let mut rest = self
            .chars
            .iter()
            .skip(self.pos.saturating_add(1))
            .peekable();
        while let Some(c) = rest.next() {
            match c {
                ' ' | '\t' | '\n' => {}
                '\\' if rest.next_if_eq(&&'\n').is_some() => {}
                _ => return *c == ')',
            }
        }
        false
    }

    /// The character at the cursor past any line continuations (`\` and a newline).
    fn peek_past_continuations(&self) -> Option<char> {
        let mut at = self.pos;
        while self.chars.get(at) == Some(&'\\')
            && self.chars.get(at.saturating_add(1)) == Some(&'\n')
        {
            at = at.saturating_add(2);
        }
        self.chars.get(at).copied()
    }

    /// Moves the cursor past any line continuations.
    fn skip_continuations(&mut self) {
        while self.peek() == Some('\\') && self.chars.get(self.pos.saturating_add(1)) == Some(&'\n')
        {
            self.advance();
            self.advance();
        }
    }

    /// Reads an extended glob's group after its `(` up to the `)` that closes it, as bash with
    /// `extglob` reads it: blanks, `|` and `;` included, quotes and `$( )` substitutions read
    /// as in any word. Without `extglob` bash refuses such a word, so reading it as a pattern
    /// can only add denials.
    ///
    /// # Errors
    /// Those of any word, [`ParseError::ProcessInExtglob`] for a `<(` or `>(` inside the
    /// group, [`ParseError::BacktickInExtglob`] for a backtick inside it at any depth, and
    /// [`ParseError::ArithmeticInExtglob`] for a `$[` inside it.
    pub(super) fn extglob(&mut self, word: &mut Word, depth: usize) -> Result<(), ParseError> {
        self.groups = self.groups.saturating_add(1);
        let read = self.group(word, depth);
        self.groups = self.groups.saturating_sub(1);
        read
    }

    fn group(&mut self, word: &mut Word, depth: usize) -> Result<(), ParseError> {
        word.push_pattern('(');
        let mut open: usize = 1;
        loop {
            let Some(c) = self.bump() else {
                return Err(ParseError::Unclosed { what: "(" });
            };
            match c {
                '(' => {
                    open = open.saturating_add(1);
                    word.text.push(c);
                }
                ')' => {
                    word.text.push(c);
                    open = open.saturating_sub(1);
                    if open == 0 {
                        return Ok(());
                    }
                }
                // Bash with `extglob` runs a process substitution inside a group, but ends the
                // group at the first `)` that balances it, which may be a `case` pattern's
                // inside the substitution; read either way, the commands differ.
                '<' | '>' if self.peek_past_continuations() == Some('(') => {
                    return Err(ParseError::ProcessInExtglob);
                }
                // Bash drops line continuations before it reads what a `$` opens, and skips a
                // `$[ ]` whole, its parentheses included, which this reader would count.
                '$' => {
                    self.skip_continuations();
                    if self.peek() == Some('[') {
                        return Err(ParseError::ArithmeticInExtglob);
                    }
                    self.word_char(c, word, depth)?;
                }
                _ => self.word_char(c, word, depth)?,
            }
        }
    }
}
