//! A pattern word where a shell or wrapper reads its options (#165): `bash -[c] 'rm -rf /'`
//! runs `bash -c` when a file named `-c` is in the directory, so which option such a word is
//! cannot be read from the command. Variables stay the word-splitting limit of #128.

use super::{Peel, SHELLS, Word, braces, readings};

/// Wrappers that read no options of their own, or whose words are patterns of their own
/// (`find -name *.rs`).
const READS_NO_OPTIONS: [&str; 9] = [
    "function", "busybox", "coproc", "eval", "alias", "source", ".", "trap", "find",
];

/// The signs a shell reads options with: `+c` is `-c`.
const SHELL_SIGNS: &[char] = &['-', '+'];

/// The sign every other wrapper reads options with.
const DASH: &[char] = &['-'];

/// The first word of `rest`, the arguments of the wrapper `name` that `peeled` removed, that
/// stands where the wrapper reads an option and is a pattern that may expand to one.
///
/// The slots are every word the wrapper took before the command it wraps, and that command's
/// first word, which the wrapper took for a program only because the pattern did not read as
/// an option. A wrapper that runs a string or a script itself has its slots read from its
/// words: for `su`, which permutes and hands its words after the user, `--` included, to the
/// user's shell, every word, signed as a shell signs them; for `script`, every word before a
/// `--`; for a shell or `env -S`, its leading options, the word after each and the first
/// operand, a shell's `-c` string ending them.
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
            before
                .iter()
                .chain(inner.first())
                .find(|word| unsure(word, DASH))
        }
        Peel::Done(_) if name == "su" => rest.iter().find(|word| unsure(word, SHELL_SIGNS)),
        Peel::Done(_) if name == "script" => rest
            .iter()
            .take_while(|word| !ends_options(word))
            .find(|word| unsure(word, DASH)),
        Peel::Done(_) if SHELLS.contains(&name) => leading(rest, SHELL_SIGNS),
        Peel::Done(_) => leading(rest, DASH),
        Peel::Program => None,
    }
}

/// The first unsure word among `words`' leading options, each option's next word (which may
/// be its value) and the first operand. After a short cluster holding `c`, the first word that
/// is no option is a shell's command string, and what follows it are operands (`$0` and the
/// rest), not options.
fn leading<'a>(words: &'a [Word], signs: &[char]) -> Option<&'a Word> {
    let mut value_may_follow = false;
    let mut string_pending = false;
    for word in words {
        if unsure(word, signs) {
            return Some(word);
        }
        let option = word.text.starts_with(signs);
        if ends_options(word) || !(option || value_may_follow) || (string_pending && !option) {
            return None;
        }
        string_pending |= option && !word.text.starts_with("--") && word.text.contains('c');
        value_may_follow = option;
    }
    None
}

/// Whether `word` holds an unquoted glob or group (a brace or `~` alone, as in `-I{}`, is
/// none) and may expand to an option signed with one of `signs`, whatever value is attached
/// (`-[c]`, `@(--)login`, `-S@(rm -rf ~)`, `-[S]'rm -rf ~'`); a word brace expansion made
/// that reads as written (`-c` from `{-c,x}`) is exactly its text.
fn unsure(word: &Word, signs: &[char]) -> bool {
    word.has_pattern()
        && word.text.contains(['*', '?', '[', '('])
        && !braces::reads_as_written(word)
        && readings::may_start_option(word, signs)
}

/// Whether `word` is a literal `--`.
fn ends_options(word: &Word) -> bool {
    word.is_literal() && word.text == "--"
}
