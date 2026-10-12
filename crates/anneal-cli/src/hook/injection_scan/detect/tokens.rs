//! Words and punctuation of scanned text, with their line and HTML comment.
//!
//! Default-ignorable characters (zero-width, bidi controls, variation selectors, fillers, tag
//! characters, soft hyphens; [`super::is_ignorable`]) are dropped without ending a word, so `ig<U+200B>nore` reads as `ignore`. Words are lowercased; an
//! apostrophe between letters stays in the word (`don't`), and `’` reads as `'`.

/// One word (a run of alphanumeric characters, lowercased) or one punctuation character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Token {
    /// The lowercased word, or the punctuation character.
    pub(super) text: String,
    /// Whether this is a word rather than punctuation.
    pub(super) word: bool,
    /// The 1-based line it starts on.
    pub(super) line: usize,
    /// The index of the HTML comment (`<!-- ... -->`) holding it, counted from 0.
    pub(super) comment: Option<usize>,
}

/// The tokens of a text, and the line each of its HTML comments opens on.
#[derive(Debug, Default)]
pub(super) struct Tokens {
    /// The tokens, in order.
    pub(super) tokens: Vec<Token>,
    /// The 1-based line of each comment's `<!--`, by comment index.
    pub(super) comment_lines: Vec<usize>,
}

/// The tokens of `text`, in order. An HTML comment left open runs to the end of the text.
pub(super) fn tokenize(text: &str) -> Tokens {
    let mut lexer = Lexer::default();
    let mut skip = 0_usize;
    for (at, c) in text.char_indices() {
        if skip > 0 {
            skip = skip.saturating_sub(1);
            continue;
        }
        // `at` comes from `char_indices`, so it is a character boundary and `get` is `Some`.
        let rest = text.get(at..).unwrap_or_default();
        if lexer.comment.is_none() && rest.starts_with("<!--") {
            lexer.flush();
            lexer.comment = Some(lexer.out.comment_lines.len());
            lexer.out.comment_lines.push(lexer.line);
            skip = 3;
        } else if lexer.comment.is_some() && rest.starts_with("-->") {
            lexer.flush();
            lexer.comment = None;
            skip = 2;
        } else if super::is_ignorable(c) {
            // Dropped without ending the word.
        } else if c.is_alphanumeric() {
            lexer.push_char(c);
        } else if is_apostrophe(c) && !lexer.word.is_empty() && next_is_alphanumeric(rest, c) {
            lexer.word.push('\'');
        } else {
            lexer.flush();
            if c == '\n' {
                lexer.line = lexer.line.saturating_add(1);
            } else if !c.is_whitespace() {
                lexer.push_punctuation(if is_apostrophe(c) { '\'' } else { c });
            }
        }
    }
    lexer.flush();
    lexer.out
}

/// `'` or `’`.
const fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}')
}

/// Whether the character after `c`, at the start of `rest`, is alphanumeric.
fn next_is_alphanumeric(rest: &str, c: char) -> bool {
    rest.get(c.len_utf8()..)
        .and_then(|after| after.chars().next())
        .is_some_and(char::is_alphanumeric)
}

/// The tokenizer's state.
#[derive(Debug)]
struct Lexer {
    out: Tokens,
    word: String,
    line: usize,
    comment: Option<usize>,
}

impl Default for Lexer {
    fn default() -> Self {
        Self {
            out: Tokens::default(),
            word: String::new(),
            line: 1,
            comment: None,
        }
    }
}

impl Lexer {
    fn push_char(&mut self, c: char) {
        self.word.extend(c.to_lowercase());
    }

    fn push_punctuation(&mut self, c: char) {
        self.out.tokens.push(Token {
            text: c.to_string(),
            word: false,
            line: self.line,
            comment: self.comment,
        });
    }

    /// Ends the word being read, if any.
    fn flush(&mut self) {
        if self.word.is_empty() {
            return;
        }
        self.out.tokens.push(Token {
            text: std::mem::take(&mut self.word),
            word: true,
            line: self.line,
            comment: self.comment,
        });
    }
}
