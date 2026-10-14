//! The strict syntax of a wrapper's value or operand slot: a word fills the slot with
//! certainty only when it has this shape, since a word that does not (`timeout x psql`, whose
//! duration `x` is not one) may be read by the wrapper as something else, which moves every
//! word after it, or be refused, so that the wrapper runs nothing. A shape therefore takes
//! only values its wrappers accept whatever the environment: no number of more than
//! [`MAX_DIGITS`] digits (`find -maxdepth 99999999999` is out of range), no count of 0 where
//! one is needed (`xargs -n 0`), no empty word (`env -C ''`).

use crate::hook::guard::shell::Word;

/// Most digits in a number a shape takes: nine digits fit every wrapper's integer (an `int`
/// at least), so none refuses one as out of range.
const MAX_DIGITS: usize = 9;

/// The syntax a value or operand must have to count as read with certainty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    /// Any word not empty and not starting with `-` (a user, a file, a pattern); attached to
    /// its option, any text but none.
    Any,
    /// ASCII digits (`xargs -P`, `ionice -n`).
    Digits,
    /// ASCII digits, not all `0` (`xargs -n`, `-L`: a count of 0 is refused).
    Count,
    /// ASCII digits after an optional `+` or `-` (`nice -n -5`, `find -mtime -7`).
    Integer,
    /// Digits with an optional `.` fraction (`watch -n 0.5`, `flock -w 1.5`, `.5`).
    Number,
    /// A [`Shape::Number`] with an optional unit of `s`, `m`, `h` or `d` (`timeout 1.5m`).
    Duration,
    /// A hexadecimal CPU mask with an optional `0x`, not all `0` (`taskset 0xff`: a mask of
    /// no CPU is refused).
    Mask,
    /// A CPU list (`taskset -c 0-3,5,8-15:2`): a digit, then digits, `,`, `-` and `:`.
    CpuList,
    /// A buffer size: digits with an optional unit of one of `kKMG` and an optional `B` or
    /// `iB` after it (`script -o 4KiB`); a larger unit may overflow.
    Size,
    /// An output buffering mode: `L` (by line) or a [`Shape::Size`] (`stdbuf -o L`; stdbuf
    /// refuses `L` for its input).
    Buffering,
    /// An environment variable's name: a word not empty and holding no `=` (`env -u a=b` is
    /// refused).
    Variable,
    /// A find size: a [`Shape::Integer`] with an optional unit of one of `bcwkMG` (`+100M`).
    Blocks,
    /// A file mode: an optional `-` or `/`, then octal digits or a symbolic mode of `ugoa`,
    /// `+-=`, `rwxXst` and `,` (`-644`, `/u+w`, `u=rwx,g=rx`); find refuses `+644`.
    Mode,
    /// One of these space-separated words (`ionice -c idle`).
    Words(&'static str),
    /// An alias definition or name (`ll=ls -l`, `ll`): a name of the characters POSIX allows
    /// in one (ASCII letters, digits, `!`, `%`, `,`, `-`, `@` and `_`), then any value after a
    /// `=`. bash defines no alias whose name it refuses (`alias 'a b=x'`).
    Alias,
}

impl Shape {
    /// Whether the word `word`, on its own after its option or as an operand, fills the slot
    /// with certainty: literal (a glob, variable or substitution may become no word or
    /// several, moving every word after it) and of this shape, or for [`Shape::Any`] not
    /// starting with `-` (it would be an option the wrapper may read as one).
    pub(super) fn fills(self, word: &Word) -> bool {
        word.is_literal()
            && match self {
                Self::Any => !word.text.is_empty() && !word.text.starts_with('-'),
                _ => self.accepts(&word.text),
            }
    }

    /// Whether `text`, a value attached to its option (`-n5`, `--signal=KILL`), has this
    /// shape; any text but none for [`Shape::Any`], since nothing else can be read from it.
    pub(super) fn accepts(self, text: &str) -> bool {
        match self {
            Self::Any => !text.is_empty(),
            Self::Digits => digits(text),
            Self::Count => digits(text) && text.bytes().any(|byte| byte != b'0'),
            Self::Integer => digits(text.strip_prefix(['+', '-']).unwrap_or(text)),
            Self::Number => number(text),
            Self::Duration => number(text.strip_suffix(['s', 'm', 'h', 'd']).unwrap_or(text)),
            Self::Mask => {
                let hex = text
                    .strip_prefix("0x")
                    .or_else(|| text.strip_prefix("0X"))
                    .unwrap_or(text);
                !hex.is_empty()
                    && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && hex.bytes().any(|byte| byte != b'0')
            }
            Self::CpuList => {
                text.starts_with(|c: char| c.is_ascii_digit())
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b',' | b'-' | b':'))
            }
            Self::Size => size(text),
            Self::Buffering => text == "L" || size(text),
            Self::Variable => !text.is_empty() && !text.contains('='),
            Self::Blocks => {
                let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
                digits(
                    unsigned
                        .strip_suffix(['b', 'c', 'w', 'k', 'M', 'G'])
                        .unwrap_or(unsigned),
                )
            }
            Self::Mode => mode(text),
            Self::Words(words) => words.split_ascii_whitespace().any(|word| word == text),
            Self::Alias => {
                let name = text.split_once('=').map_or(text, |(name, _)| name);
                !name.is_empty()
                    && name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric()
                            || matches!(byte, b'!' | b'%' | b',' | b'-' | b'@' | b'_')
                    })
            }
        }
    }
}

/// Whether `text` is one to [`MAX_DIGITS`] ASCII digits.
fn digits(text: &str) -> bool {
    !text.is_empty() && at_most_digits(text)
}

/// Whether `text` is at most [`MAX_DIGITS`] ASCII digits, none included.
fn at_most_digits(text: &str) -> bool {
    text.len() <= MAX_DIGITS && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether `text` is digits with an optional `.` fraction, holding at least one digit and at
/// most [`MAX_DIGITS`] on either side.
fn number(text: &str) -> bool {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    (digits(whole) || digits(fraction)) && at_most_digits(whole) && at_most_digits(fraction)
}

/// Whether `text` is a buffer size of digits and an optional unit ([`Shape::Size`]).
fn size(text: &str) -> bool {
    let number_end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at_checked(number_end).unwrap_or((text, ""));
    let unit_ok = match unit.as_bytes() {
        [] => true,
        [prefix, rest @ ..] => {
            b"kKMG".contains(prefix) && matches!(rest, [] | [b'B'] | [b'i', b'B'])
        }
    };
    digits(number) && unit_ok
}

/// Whether `text` is a file mode ([`Shape::Mode`]).
fn mode(text: &str) -> bool {
    let bare = text.strip_prefix(['-', '/']).unwrap_or(text);
    let octal = !bare.is_empty() && bare.bytes().all(|byte| matches!(byte, b'0'..=b'7'));
    let symbolic = bare.contains(['+', '-', '='])
        && bare.bytes().all(|byte| b"ugoa+-=rwxXst,".contains(&byte));
    octal || symbolic
}

#[cfg(test)]
mod tests;
