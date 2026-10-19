//! A pattern word where a shell or wrapper reads its options (#165): `bash -[c] 'rm -rf /'`
//! runs `bash -c` when a file named `-c` is in the directory, so which option such a word is
//! cannot be read from the command. Variables stay the word-splitting limit of #128.

use super::{Peel, Word, braces, readings};

/// Wrappers that read no options of their own, or whose words are patterns of their own
/// (`find -name *.rs`).
const READS_NO_OPTIONS: [&str; 9] = [
    "function", "busybox", "coproc", "eval", "alias", "source", ".", "trap", "find",
];

/// Wrappers that read their options anywhere before a `--` (`su root -c cmd`).
const PERMUTE: [&str; 2] = ["su", "script"];

/// The first word of `rest`, the arguments of the wrapper `name` that `peeled` removed, that
/// stands where the wrapper reads an option and is a pattern that may expand to one.
///
/// The slots are every word the wrapper took before the command it wraps, and that command's
/// first word, which the wrapper took for a program only because the pattern did not read as
/// an option. A wrapper that runs a string or a script itself (a shell, `su`, `env -S`) has
/// its slots read from its words: all of them before a `--` for one that permutes, else its
/// leading options and the word after each, up to its first operand.
pub(super) fn unsure_option<'a>(
    name: &str,
    rest: &'a [Word],
    peeled: &Peel<'a>,
) -> Option<&'a Word> {
    if READS_NO_OPTIONS.contains(&name) {
        return None;
    }
    match peeled {
        Peel::Inner(inner) | Peel::Appending(inner) => {
            let taken = rest.len().saturating_sub(inner.len());
            let before = rest
                .split_at_checked(taken)
                .map_or(rest, |(before, _)| before);
            before.iter().chain(inner.first()).find(|word| unsure(word))
        }
        Peel::Done(_) if PERMUTE.contains(&name) => rest
            .iter()
            .take_while(|word| !ends_options(word))
            .find(|word| unsure(word)),
        Peel::Done(_) => leading(rest),
        Peel::Program => None,
    }
}

/// The first unsure word among `words`' leading options, each option's next word (which may
/// be its value) and the first operand.
fn leading(words: &[Word]) -> Option<&Word> {
    let mut value_may_follow = false;
    for word in words {
        if unsure(word) {
            return Some(word);
        }
        if ends_options(word) {
            return None;
        }
        let option = word.text.starts_with('-');
        if !(option || value_may_follow) {
            return None;
        }
        value_may_follow = option;
    }
    None
}

/// Whether `word` holds an unquoted pattern character and may expand to an option cluster, or
/// is an option written out whose name or value goes on as a pattern (`-S'rm -rf /'*`,
/// `--split-string=@(x)`); a word brace expansion made that reads as written (`-c` from
/// `{-c,x}`) is exactly its text.
fn unsure(word: &Word) -> bool {
    word.has_pattern()
        && !braces::reads_as_written(word)
        && (readings::may_start_option_cluster(word) || written_option(&word.text))
}

/// Whether `text` starts with a written `-` and a letter, digit or second `-` (an expansion's
/// source text starts with `$` or a backtick, and a glob with `?`, `*`, `[` or a group's
/// opener) and holds a glob or a group: a brace or `~` alone (`-I{}`) is no glob.
fn written_option(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next() == Some('-')
        && chars
            .next()
            .is_some_and(|c| c == '-' || c.is_ascii_alphanumeric())
        && text.contains(['*', '?', '[', '('])
}

/// Whether `word` is a literal `--`.
fn ends_options(word: &Word) -> bool {
    word.is_literal() && word.text == "--"
}
