//! Success claims in a reply: the words that say the work is finished or the tests pass.
//!
//! A claim is one of [`SUCCESS_PHRASES`] as whole words, in any letter case, outside fenced
//! code blocks, code spans and HTML comments (quoted output is not a claim), and not right
//! after a negation (`not`, `never`, `no`, or a word ending in `n't`, one or two words before).

use crate::lint::spec::markdown::{Document, without_code_spans};

/// The phrases that claim success, as lowercase words, longest first so the most specific one
/// is reported.
pub(crate) const SUCCESS_PHRASES: [&str; 7] = [
    "all tests pass",
    "tests pass",
    "works now",
    "done",
    "passing",
    "green",
    "fixed",
];

/// Words that negate a phrase that follows within two words.
const NEGATIONS: [&str; 3] = ["not", "never", "no"];

/// The first of [`SUCCESS_PHRASES`] that `reply` claims, if any.
pub(crate) fn success_claim(reply: &str) -> Option<&'static str> {
    let document = Document::parse(reply);
    let words: Vec<String> = document
        .lines()
        .iter()
        .flat_map(|line| words(&without_code_spans(&line.text)))
        .collect();
    SUCCESS_PHRASES
        .into_iter()
        .find(|phrase| claims(&words, phrase))
}

/// True when `phrase` appears in `words` as whole words, not negated.
fn claims(words: &[String], phrase: &str) -> bool {
    let wanted: Vec<&str> = phrase.split(' ').collect();
    words
        .windows(wanted.len())
        .enumerate()
        .any(|(start, window)| window.iter().eq(wanted.iter()) && !negated(words, start))
}

/// True when one of the two words before `start` is a negation.
fn negated(words: &[String], start: usize) -> bool {
    let from = start.saturating_sub(2);
    words.get(from..start).is_some_and(|before| {
        before
            .iter()
            .any(|word| NEGATIONS.contains(&word.as_str()) || word.ends_with("n't"))
    })
}

/// The lowercase words of `text`: runs of letters and digits, with an apostrophe (`'` or `’`,
/// read as `'`) kept inside a word, so `isn't` is one word.
fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else if matches!(c, '\'' | '\u{2019}') && !word.is_empty() {
            word.push('\'');
        } else if !word.is_empty() {
            words.push(finished(std::mem::take(&mut word)));
        }
    }
    if !word.is_empty() {
        words.push(finished(word));
    }
    words
}

/// `word` without the apostrophes at its end (a closing quote, not part of the word).
fn finished(mut word: String) -> String {
    while word.ends_with('\'') {
        word.pop();
    }
    word
}

#[cfg(test)]
mod tests {
    use super::success_claim;

    #[test]
    fn each_success_phrase_is_a_claim_in_any_letter_case() {
        let replies = [
            ("All Tests Pass now.", "all tests pass"),
            ("The tests pass.", "tests pass"),
            ("It works now!", "works now"),
            ("Done.", "done"),
            ("Everything is passing", "passing"),
            ("CI is GREEN", "green"),
            ("Fixed the parser.", "fixed"),
        ];
        for (reply, phrase) in replies {
            assert_eq!(success_claim(reply), Some(phrase), "{reply:?}");
        }
    }

    #[test]
    fn only_whole_words_count() {
        for reply in [
            "I abandoned the branch",
            "the greenfield rewrite",
            "a prefixed name",
            "undone work",
            "the tests passed",
        ] {
            assert_eq!(success_claim(reply), None, "{reply:?}");
        }
    }

    #[test]
    fn words_inside_fenced_code_blocks_and_code_spans_are_not_claims() {
        let reply = "Here is the log:\n\n```\ntest result: ok. all tests pass\ndone\n```\n\n\
                     Run `cargo test --color green` to see it.";

        assert_eq!(success_claim(reply), None, "code is quoted, not claimed");
    }

    #[test]
    fn a_negated_phrase_is_not_a_claim() {
        for reply in [
            "The tests are not passing yet.",
            "This isn't done.",
            "It is not yet fixed.",
            "Not all tests pass.",
        ] {
            assert_eq!(success_claim(reply), None, "{reply:?}");
        }
    }

    #[test]
    fn the_longest_phrase_is_reported() {
        assert_eq!(
            success_claim("ok, all tests pass and it works now"),
            Some("all tests pass"),
            "the first phrase in list order"
        );
    }
}
