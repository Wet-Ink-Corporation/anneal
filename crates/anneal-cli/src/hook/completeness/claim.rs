//! Success claims in a reply: the words that say the work is finished or the tests pass.
//!
//! A claim is one of [`CLAIMS`], each word in one of its listed forms (`pass`, `passes`,
//! `passed`, `passing`), as whole words in one clause, in any letter case, outside fenced code
//! blocks, code spans and HTML comments (quoted output is not a claim), and not right after a
//! negation (`not`, `never`, `no`, or a word ending in `n't`, one or two words before in the
//! same clause). Clauses end at `.`, `!`, `?`, `;`, `:`, `,`, a dash and a line end, so in
//! "No failures. Done." the "No" does not reach "Done". False positives ("done reading the
//! file") are expected; the hook's block cap bounds what they cost.

use crate::lint::spec::markdown::{Document, without_code_spans};

/// The forms one word of a claim may take, lowercase.
type Forms = &'static [&'static str];

/// `pass` in every form, after `tests`.
const PASS: Forms = &["pass", "passes", "passed", "passing"];

/// `test` or `tests`.
const TESTS: Forms = &["tests", "test"];

/// The claims, longest first so the most specific one is reported.
const CLAIMS: [&[Forms]; 11] = [
    &[&["all"], TESTS, PASS],
    &[TESTS, PASS],
    &[&["works", "work"], &["now"]],
    &[&["all"], &["green"]],
    &[&["done"]],
    // Bare `pass` is left out: "pass the flag" is not a claim.
    &[&["passes", "passed", "passing"]],
    &[&["green"]],
    &[&["fixed"]],
    &[&["complete", "completed"]],
    &[&["finished"]],
    &[&["succeeded", "succeeds"]],
];

/// Words that negate a claim that follows within two words.
const NEGATIONS: [&str; 3] = ["not", "never", "no"];

/// Characters that end a clause.
const CLAUSE_ENDS: [char; 9] = ['.', '!', '?', ';', ':', ',', '-', '\u{2013}', '\u{2014}'];

/// The first of [`CLAIMS`] that `reply` makes, as written there (lowercase), if any.
pub(crate) fn success_claim(reply: &str) -> Option<String> {
    let document = Document::parse(reply);
    let clauses: Vec<Vec<String>> = document
        .lines()
        .iter()
        .flat_map(|line| clauses(&without_code_spans(&line.text)))
        .collect();
    CLAIMS
        .into_iter()
        .find_map(|claim| clauses.iter().find_map(|words| made(words, claim)))
}

/// The words of `claim` in `words`, joined, when they are there and not negated.
fn made(words: &[String], claim: &[Forms]) -> Option<String> {
    words
        .windows(claim.len())
        .enumerate()
        .find(|(start, window)| {
            window
                .iter()
                .zip(claim)
                .all(|(word, forms)| forms.contains(&word.as_str()))
                && !negated(words, *start)
        })
        .map(|(_, window)| window.join(" "))
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

/// The clauses of `text`, each its lowercase words: runs of letters and digits, with an
/// apostrophe (`'` or `’`, read as `'`) kept inside a word, so `isn't` is one word. Empty
/// clauses are dropped.
fn clauses(text: &str) -> Vec<Vec<String>> {
    let mut clauses = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else if matches!(c, '\'' | '\u{2019}') && !word.is_empty() {
            word.push('\'');
        } else {
            if !word.is_empty() {
                words.push(finished(std::mem::take(&mut word)));
            }
            if CLAUSE_ENDS.contains(&c) && !words.is_empty() {
                clauses.push(std::mem::take(&mut words));
            }
        }
    }
    if !word.is_empty() {
        words.push(finished(word));
    }
    if !words.is_empty() {
        clauses.push(words);
    }
    clauses
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
            assert_eq!(success_claim(reply).as_deref(), Some(phrase), "{reply:?}");
        }
    }

    #[test]
    fn inflections_and_other_finishing_words_are_claims_as_written() {
        let replies = [
            ("All tests passed.", "all tests passed"),
            ("the tests passed", "tests passed"),
            ("Everything passes.", "passes"),
            ("Tests: 40 passed, 0 failed.", "passed"),
            ("The implementation is complete.", "complete"),
            ("Migration completed", "completed"),
            ("Finished: everything works.", "finished"),
            ("The build succeeded.", "succeeded"),
            ("It succeeds on every target.", "succeeds"),
            ("All green.", "all green"),
            ("The test passes now", "test passes"),
        ];
        for (reply, claim) in replies {
            assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
        }
    }

    #[test]
    fn only_whole_words_count() {
        for reply in [
            "I abandoned the branch",
            "the greenfield rewrite",
            "a prefixed name",
            "undone work",
            "an incomplete list",
            "a password and a passport",
        ] {
            assert_eq!(success_claim(reply), None, "{reply:?}");
        }
    }

    #[test]
    fn a_negation_reaches_only_within_its_clause() {
        for reply in [
            "No failures. Done.",
            "No warnings, no failures. Done.",
            "Nothing is left to change; done",
            "Not much changed: all tests pass",
            "It was not hard - fixed",
            "Never mind that\nDone",
        ] {
            assert!(success_claim(reply).is_some(), "{reply:?}");
        }
        for reply in ["not done", "It isn't fixed.", "The tests have not passed"] {
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
            success_claim("ok, all tests pass and it works now").as_deref(),
            Some("all tests pass"),
            "the first phrase in list order"
        );
    }
}
