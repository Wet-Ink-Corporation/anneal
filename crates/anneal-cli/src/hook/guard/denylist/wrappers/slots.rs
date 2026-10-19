//! A pattern word where a shell or wrapper reads its options (#165): `bash -[c] 'rm -rf /'`
//! runs `bash -c` when a file named `-c` is in the directory, so which option such a word is
//! cannot be read from the command. Variables stay the word-splitting limit of #128.

use super::grammar::Grammar;
use super::strings::{ends_options, shell_option_words};
use super::tables::{ENV, FLOCK, SUDO, WATCH};
use super::{Peel, SHELLS, Word, braces, readings};

/// Wrappers that read no options of their own, or whose words are patterns of their own
/// (`find -name *.rs`).
const READS_NO_OPTIONS: [&str; 9] = [
    "function", "busybox", "coproc", "eval", "alias", "source", ".", "trap", "find",
];

/// How a program reads an option word.
struct Reader {
    /// The signs that start an option.
    signs: &'static [char],
    /// Whether an option may carry its value in the same word (`-uroot`).
    attached: bool,
}

/// A shell: `+c` is `-c`, and its options carry no value in their word.
const SHELL: Reader = Reader {
    signs: &['-', '+'],
    attached: false,
};

/// Every other wrapper's getopt.
const GETOPT: Reader = Reader {
    signs: &['-'],
    attached: true,
};

/// `su`, whose words are read by its getopt and by the user's shell.
const SU: Reader = Reader {
    signs: &['-', '+'],
    attached: true,
};

/// The first word of `rest`, the arguments of the wrapper `name` that `peeled` removed, that
/// stands where the wrapper reads an option and is a pattern that may expand to one.
///
/// The slots are every word the wrapper took before the command it wraps, and that command's
/// first word, which the wrapper took for a program only because the pattern did not read as
/// an option. A wrapper that runs a string or a script itself has its slots read from its
/// words: for a shell, the words it reads as options or their values and the word that ends
/// them, as [`shell_option_words`] reads them; for `su`, which permutes and hands its words
/// after the user, `--` included, to the user's shell, every word, signed as a shell signs
/// them; for `script`, every word before a `--`; for `sudo -s`, `env -S`, `watch` and
/// `flock -c`, the words their grammar reads as options and the word after them.
pub(super) fn unsure_option<'a>(
    name: &str,
    rest: &'a [Word],
    peeled: &Peel<'a>,
) -> Option<&'a Word> {
    if READS_NO_OPTIONS.contains(&name) {
        return None;
    }
    let slots = match peeled {
        Peel::Program => return None,
        Peel::Inner(inner) | Peel::Appending(inner) => {
            let taken = rest.len().saturating_sub(inner.len());
            return before_and_first(rest, taken).find(|word| unsure(word, &GETOPT));
        }
        Peel::Done(_) if name == "su" => {
            return rest.iter().find(|word| unsure(word, &SU));
        }
        Peel::Done(_) if SHELLS.contains(&name) => {
            let read = shell_option_words(rest);
            return rest.iter().take(read).find(|word| unsure(word, &SHELL));
        }
        Peel::Done(_) if name == "script" => {
            rest.iter().position(ends_options).unwrap_or(rest.len())
        }
        Peel::Done(_) => grammar(name).map_or(rest.len(), |grammar| {
            rest.len().saturating_sub(grammar.parse(rest).rest.len())
        }),
    };
    before_and_first(rest, slots).find(|word| unsure(word, &GETOPT))
}

/// The grammar of a wrapper that may run a string itself.
fn grammar(name: &str) -> Option<&'static Grammar> {
    match name {
        "sudo" | "doas" => Some(&SUDO),
        "env" => Some(&ENV),
        "watch" => Some(&WATCH),
        "flock" => Some(&FLOCK),
        _ => None,
    }
}

/// The first `taken` words of `words` and the word after them, unless the last of them is a
/// literal `--`, after which getopt reads no option (`sudo -- *.sh` runs a command).
fn before_and_first(words: &[Word], taken: usize) -> impl Iterator<Item = &Word> {
    let (before, after) = words.split_at_checked(taken).unwrap_or((words, &[]));
    let first = if before.last().is_some_and(ends_options) {
        None
    } else {
        after.first()
    };
    before.iter().chain(first)
}

/// Whether `word` holds an unquoted glob or group (a brace or `~` alone, as in `-I{}`, is
/// none) and may expand to an option as `reader` reads one, whatever value is attached
/// (`-[c]`, `@(--)login`, `-S@(rm -rf ~)`, `-[S]'rm -rf ~'`, `*' rm -rf ~'`); a word brace
/// expansion made that reads as written (`-c` from `{-c,x}`) is exactly its text.
fn unsure(word: &Word, reader: &Reader) -> bool {
    word.has_pattern()
        && word.text.contains(['*', '?', '[', '('])
        && !braces::reads_as_written(word)
        && readings::may_start_option(word, reader.signs, reader.attached)
}
