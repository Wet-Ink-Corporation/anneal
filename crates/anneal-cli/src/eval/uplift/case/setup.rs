//! A case's setup command: an argument vector its fixture's scaffold runs after the commit,
//! never a shell string, so each word reaches the command exactly as written.

/// The most setup commands a case may list.
pub(crate) const MAX_COMMANDS: usize = 8;
/// The most words in one command, its program included.
const MAX_WORDS: usize = 32;
/// The longest word, in bytes.
const MAX_WORD_BYTES: usize = 512;

/// One setup command: 1 to 32 words, the first not blank, each at most 512 bytes and free of
/// control characters (no NUL, which no argument can hold, and no line break in the script).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Argv(Vec<String>);

/// Why a setup command is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ArgvFault {
    /// It has no word.
    #[error("the command is empty")]
    Empty,
    /// Its program, the first word, is blank.
    #[error("the program is blank")]
    BlankProgram,
    /// It has more than 32 words.
    #[error("the command has more than {MAX_WORDS} words")]
    TooManyWords,
    /// A word is longer than 512 bytes.
    #[error("word {0} is longer than {MAX_WORD_BYTES} bytes")]
    LongWord(usize),
    /// A word holds a control character.
    #[error("word {0} holds a control character")]
    ControlCharacter(usize),
}

impl Argv {
    /// Its words, the program first.
    pub(crate) fn words(&self) -> &[String] {
        &self.0
    }
}

impl TryFrom<Vec<String>> for Argv {
    type Error = ArgvFault;

    /// The command `words`, words counted from 1 in a fault.
    fn try_from(words: Vec<String>) -> Result<Self, ArgvFault> {
        let Some(program) = words.first() else {
            return Err(ArgvFault::Empty);
        };
        if program.trim().is_empty() {
            return Err(ArgvFault::BlankProgram);
        }
        if words.len() > MAX_WORDS {
            return Err(ArgvFault::TooManyWords);
        }
        for (index, word) in (1..).zip(&words) {
            if word.len() > MAX_WORD_BYTES {
                return Err(ArgvFault::LongWord(index));
            }
            if word.chars().any(char::is_control) {
                return Err(ArgvFault::ControlCharacter(index));
            }
        }
        Ok(Self(words))
    }
}

#[cfg(test)]
mod tests;
