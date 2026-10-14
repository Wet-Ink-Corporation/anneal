//! Success claims in a reply: the words that say the work is finished or the tests pass.
//!
//! A claim is one of [`CLAIMS`], each word in one of its listed forms (`pass`, `passes`,
//! `passed`, `passing`), as whole words in one clause, in any letter case, outside fenced code
//! blocks, code spans and HTML comments (quoted output is not a claim). A claim is withdrawn
//! by three structural rules, each within its clause: a negation (`not`, `never`, `no`, or a
//! word ending in `n't`) one or two words before it, a conditional ([`CONDITIONALS`]:
//! "once checks pass") anywhere before it, and a question mark ending the clause ("Does it
//! work?"). Clauses end at `.`, `!`, `?`, `;`, `:`, `,`, a dash and a line end, so in
//! "No failures. Done." the "No" does not reach "Done". A hyphen ends a clause only when it is
//! not between two letters or digits: `half-done` is one word (not `done`), while
//! `hard - fixed` and `hard--fixed` are two clauses. False positives ("done reading the
//! file", "how it works") are expected; the hook's block cap bounds what they cost.

use crate::lint::spec::markdown::{Document, without_code_spans};

/// The forms one word of a claim may take, lowercase.
type Forms = &'static [&'static str];

/// `pass` in every form, after a [`SUITES`] word.
const PASS: Forms = &["pass", "passes", "passed", "passing"];

/// What passes: tests, checks or builds.
const SUITES: Forms = &["tests", "test", "checks", "check", "builds", "build"];

/// Who works in a "works" claim.
const WORKER: Forms = &["everything", "it"];

/// The claims in report order: the first one a reply makes is reported. The phrases from #26
/// come first, longest first, so their reports are unchanged; the #84 additions follow.
const CLAIMS: [&[Forms]; 17] = [
    &[&["all"], SUITES, PASS],
    &[SUITES, PASS],
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
    &[&["ready"], &["to"], &["merge"]],
    &[&["ready"], &["for"], &["review"]],
    &[WORKER, &["is"], &["working"]],
    &[
        &["everything", "it", "everything's", "it's"],
        &["works", "working"],
    ],
    &[&["implemented"]],
    &[&["resolved"]],
];

/// Words that negate a claim that follows within two words.
const NEGATIONS: [&str; 3] = ["not", "never", "no"];

/// Words that make what follows them in their clause a condition, not a claim.
const CONDITIONALS: [&str; 6] = ["if", "once", "when", "whenever", "unless", "until"];

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
                && !conditional(words, *start)
        })
        .map(|(_, window)| window.join(" "))
}

/// True when a conditional word comes anywhere before `start`.
fn conditional(words: &[String], start: usize) -> bool {
    words.get(..start).is_some_and(|before| {
        before
            .iter()
            .any(|word| CONDITIONALS.contains(&word.as_str()))
    })
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

/// The clauses of `text` that are not questions, each its lowercase words: runs of letters and
/// digits, with an apostrophe (`'` or `’`, read as `'`) kept inside a word, so `isn't` is one
/// word, and a hyphen between two letters or digits kept too, so `half-done` is one word.
/// Empty clauses, and clauses a `?` ends, are dropped.
fn clauses(text: &str) -> Vec<Vec<String>> {
    let mut clauses = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else if matches!(c, '\'' | '\u{2019}') && !word.is_empty() {
            word.push('\'');
        } else if c == '-'
            && word.chars().next_back().is_some_and(char::is_alphanumeric)
            && chars.peek().is_some_and(|next| next.is_alphanumeric())
        {
            word.push('-');
        } else {
            if !word.is_empty() {
                words.push(finished(std::mem::take(&mut word)));
            }
            if CLAUSE_ENDS.contains(&c) {
                let clause = std::mem::take(&mut words);
                if !clause.is_empty() && c != '?' {
                    clauses.push(clause);
                }
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
    fn each_n1_reply_is_a_claim() {
        let replies = [
            ("Everything works.", "everything works"),
            ("The feature is implemented and working.", "implemented"),
            ("Ready to merge.", "ready to merge"),
            ("Resolved.", "resolved"),
            ("All checks pass.", "all checks pass"),
            ("The build passes", "build passes"),
            ("It's working", "it's working"),
            ("Everything is working now.", "everything is working"),
            ("Ready for review", "ready for review"),
        ];
        for (reply, claim) in replies {
            assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
        }
    }

    #[test]
    fn the_new_words_do_not_fire_on_ordinary_non_claims() {
        for reply in [
            "Does it work?",
            "Not implemented yet.",
            "Once checks pass, I'll merge.",
            "It isn't working.",
            "Tests should pass after the fix.",
            "Not everything works.",
            "It is not ready to merge.",
            "This isn't resolved",
        ] {
            assert_eq!(success_claim(reply), None, "{reply:?}");
        }
    }

    #[test]
    fn a_question_is_not_a_claim() {
        for reply in [
            "Is everything working?",
            "Does it work now?",
            "Did the tests pass?",
            "Ready to merge?",
        ] {
            assert_eq!(success_claim(reply), None, "{reply:?}");
        }
        assert_eq!(
            success_claim("All tests pass - want me to commit?").as_deref(),
            Some("all tests pass"),
            "only the clause the question mark ends is a question"
        );
    }

    #[test]
    fn a_conditional_before_a_claim_withdraws_it() {
        for reply in [
            "I'll merge once checks pass.",
            "If it works, I'll tag it.",
            "When the build passes, I'll tag it.",
            "Keep going until all tests pass",
            "Unless it's resolved upstream, we patch it.",
        ] {
            assert_eq!(success_claim(reply), None, "{reply:?}");
        }
        assert_eq!(
            success_claim("If it helps, all tests pass.").as_deref(),
            Some("all tests pass"),
            "a conditional reaches only within its clause"
        );
        assert_eq!(
            success_claim("All tests pass when run alone.").as_deref(),
            Some("all tests pass"),
            "a conditional after the claim does not withdraw it"
        );
    }

    #[test]
    fn an_in_word_hyphen_does_not_end_a_clause() {
        for reply in [
            "It's half-done, not finished.",
            "The parser is well-tested.",
            "A pre-fixed list",
            "Not re-done",
        ] {
            assert_eq!(success_claim(reply), None, "{reply:?}");
        }
    }

    #[test]
    fn a_spaced_hyphen_double_hyphen_and_dashes_still_end_a_clause() {
        for reply in [
            "It was not hard - fixed",
            "It was not hard--fixed",
            "It was not hard -fixed",
            "It was not hard- fixed",
            "It was not hard \u{2013} fixed",
            "It was not hard\u{2014}fixed",
        ] {
            assert_eq!(success_claim(reply).as_deref(), Some("fixed"), "{reply:?}");
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
