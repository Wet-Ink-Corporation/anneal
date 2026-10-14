//! The strict syntax of a wrapper's value or operand slot: a word fills the slot with
//! certainty only when it has this shape, since a word that does not (`chrt -i mysql`, whose
//! priority util-linux 2.42 takes only when all digits) may be read by the wrapper as
//! something else, which moves every word after it.

use crate::hook::guard::shell::Word;

/// The syntax a value or operand must have to count as read with certainty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    /// Any word not starting with `-` (a user, a file, a pattern); attached to its option,
    /// any text.
    Any,
    /// ASCII digits (`chrt`'s priority, `xargs -n`).
    Digits,
    /// ASCII digits after an optional `+` or `-` (`nice -n -5`, `find -mtime -7`).
    Integer,
    /// Digits with an optional `.` fraction (`watch -n 0.5`, `flock -w 1.5`, `.5`).
    Number,
    /// A [`Shape::Number`] with an optional unit of `s`, `m`, `h` or `d` (`timeout 1.5m`).
    Duration,
    /// A signal number, or a name of ASCII letters with an optional `+` or `-` and digits
    /// (`KILL`, `SIGTERM`, `RTMIN+1`).
    Signal,
    /// A hexadecimal CPU mask with an optional `0x` (`taskset 0xff`).
    Mask,
    /// A CPU list (`taskset -c 0-3,5,8-15:2`): a digit, then digits, `,`, `-` and `:`.
    CpuList,
    /// A buffer size: `L`, or digits with an optional unit of one of `kKMGTPEZYRQ` and an
    /// optional `B` or `iB` after it (`stdbuf -o 4KiB`).
    Size,
    /// A find size: a [`Shape::Integer`] with an optional unit of one of `bcwkMG` (`+100M`).
    Blocks,
    /// A file mode: an optional `-`, `/` or `+`, then octal digits or a symbolic mode of
    /// `ugoa`, `+-=`, `rwxXst` and `,` (`-644`, `/u+w`, `u=rwx,g=rx`).
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
                Self::Any => !word.text.starts_with('-'),
                _ => self.accepts(&word.text),
            }
    }

    /// Whether `text`, a value attached to its option (`-n5`, `--signal=KILL`), has this
    /// shape; any text for [`Shape::Any`], since nothing else can be read from it.
    pub(super) fn accepts(self, text: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Digits => digits(text),
            Self::Integer => digits(text.strip_prefix(['+', '-']).unwrap_or(text)),
            Self::Number => number(text),
            Self::Duration => number(text.strip_suffix(['s', 'm', 'h', 'd']).unwrap_or(text)),
            Self::Signal => signal(text),
            Self::Mask => {
                let hex = text
                    .strip_prefix("0x")
                    .or_else(|| text.strip_prefix("0X"))
                    .unwrap_or(text);
                !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
            }
            Self::CpuList => {
                text.starts_with(|c: char| c.is_ascii_digit())
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b',' | b'-' | b':'))
            }
            Self::Size => text == "L" || size(text),
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

/// Whether `text` is one or more ASCII digits.
fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether `text` is digits with an optional `.` fraction, holding at least one digit.
fn number(text: &str) -> bool {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    (digits(whole) || digits(fraction)) && all_digits(whole) && all_digits(fraction)
}

/// Whether `text` is a signal number or name ([`Shape::Signal`]).
fn signal(text: &str) -> bool {
    if digits(text) {
        return true;
    }
    let letters = text
        .find(['+', '-'])
        .map_or(text, |sign| text.get(..sign).unwrap_or(text));
    let offset = text.get(letters.len()..).unwrap_or("");
    !letters.is_empty()
        && letters.bytes().all(|byte| byte.is_ascii_alphabetic())
        && (offset.is_empty() || digits(offset.get(1..).unwrap_or("")))
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
            b"kKMGTPEZYRQ".contains(prefix) && matches!(rest, [] | [b'B'] | [b'i', b'B'])
        }
    };
    digits(number) && unit_ok
}

/// Whether `text` is a file mode ([`Shape::Mode`]).
fn mode(text: &str) -> bool {
    let bare = text.strip_prefix(['-', '/', '+']).unwrap_or(text);
    let octal = !bare.is_empty() && bare.bytes().all(|byte| matches!(byte, b'0'..=b'7'));
    let symbolic = bare.contains(['+', '-', '='])
        && bare.bytes().all(|byte| b"ugoa+-=rwxXst,".contains(&byte));
    octal || symbolic
}

#[cfg(test)]
mod tests;
