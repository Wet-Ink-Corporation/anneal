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
    /// `()` is the definition's.
    Name,
    /// An argument or a redirection's target: `!(x)` is an extended glob.
    Argument,
}

impl Place {
    /// Whether a function's name may stand here, so a `()` after it defines the function.
    fn names_function(self) -> bool {
        matches!(self, Self::Command | Self::Name)
    }
}

/// The characters that open an extended glob when an unquoted `(` follows them.
pub(crate) const OPENERS: [char; 5] = ['@', '+', '!', '?', '*'];

/// Whether a `(` after `word` opens an extended glob's group: the word ends with an unquoted
/// [`OPENERS`] character that is not an expansion's (`$@`), and that character is not a `!`
/// standing alone in command position.
fn opens(word: &Word, place: Place) -> bool {
    let len = word.text.len();
    let unquoted_end = word.quotes.last().is_none_or(|quote| quote.end < len)
        && word
            .expansions
            .last()
            .is_none_or(|expansion| expansion.range.end < len);
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
    /// Whether the `(` at the cursor opens an extended glob's group after `word`. Where a
    /// function's name may stand, an empty `()` (blanks allowed: `f?( )`) is the definition's,
    /// which bash accepts without `extglob`; elsewhere `@()` is an empty group.
    pub(super) fn opens_extglob(&self, word: &Word, place: Place) -> bool {
        self.peek() == Some('(')
            && opens(word, place)
            && !(place.names_function() && self.empty_parens())
    }

    /// Whether the `(` at the cursor is followed, past blanks and newlines, by `)`.
    fn empty_parens(&self) -> bool {
        self.chars
            .iter()
            .skip(self.pos.saturating_add(1))
            .find(|c| !matches!(c, ' ' | '\t' | '\n'))
            == Some(&')')
    }

    /// Reads an extended glob's group after its `(` up to the `)` that closes it, as bash with
    /// `extglob` reads it: blanks, `|` and `;` included, quotes and substitutions read as in
    /// any word. Without `extglob` bash refuses such a word, so reading it as a pattern can
    /// only add denials.
    pub(super) fn extglob(&mut self, word: &mut Word, depth: usize) -> Result<(), ParseError> {
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
                '<' | '>' if self.peek() == Some('(') => {
                    return Err(ParseError::ProcessInExtglob);
                }
                _ => self.word_char(c, word, depth)?,
            }
        }
    }
}
