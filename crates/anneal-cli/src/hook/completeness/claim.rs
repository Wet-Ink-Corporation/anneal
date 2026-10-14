//! Success claims in a reply: the words that say the work is finished or the tests pass.
//!
//! A claim is one of [`CLAIMS`], each word in one of its listed forms (`pass`, `passes`,
//! `passed`, `passing`), as whole words in one clause, in any letter case, outside fenced code
//! blocks, code spans and HTML comments (quoted output is not a claim). A claim is withdrawn
//! by three structural rules, each within its clause: a negation (`not`, `never`, `no`, or a
//! word ending in `n't`) one or two words before it, a conditional ([`CONDITIONALS`]) one or
//! two words before it ("I'll merge once checks pass") or opening its clause ("When the build
//! passes"), and a question mark ending the clause ("Does it work?"). A conditional further
//! back governs a phrase of its own, so "The crash when saving is fixed." claims. Clauses end
//! at `.`, `!`, `?`, `;`, `:`, `,`, a dash and a line end, so in "No failures. Done." the "No"
//! does not reach "Done", and in "Did the tests pass, or should I re-run?" only the second
//! clause is a question. A hyphen ends a clause only when it is not between two letters or
//! digits: `half-done` is one word (not `done`), while `hard - fixed` and `hard--fixed` are
//! two clauses; a hyphenated word whose last segment is a suite word (`unit-tests`) is a suite.
//! False positives ("done reading the file", "how it works") are expected; the hook's block
//! cap bounds what they cost.

use crate::lint::spec::markdown::{Document, without_code_spans};

/// The forms one word of a claim may take, lowercase.
type Forms = &'static [&'static str];

/// `pass` in every form, after a [`SUITES`] word.
const PASS: Forms = &["pass", "passes", "passed", "passing"];

/// What passes: tests, checks or builds.
const SUITES: Forms = &["tests", "test", "checks", "check", "builds", "build"];

/// Who works in a "works" claim.
const WORKER: Forms = &["everything", "it"];

/// Who works in a "works" claim, contractions included.
const WORKER_OR_CONTRACTION: Forms = &["everything", "it", "everything's", "it's"];

/// `works` in the forms a "works" claim takes.
const WORKS: Forms = &["works", "working"];

/// The claims in report order: the first one a reply makes is reported. The phrases from #26
/// come first, longest first, and the #84 additions follow, so a reply making a #26 claim
/// still reports that claim (unless a structural rule now withdraws it).
const CLAIMS: [&[Forms]; 18] = [
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
    &[WORKER_OR_CONTRACTION, WORKS],
    &[&["implemented"]],
    &[&["resolved"]],
    // One adverb between subject and verb: "Everything still works.", "It all works."
    &[WORKER_OR_CONTRACTION, &["still", "now", "all"], WORKS],
];

/// Words that negate a claim that follows within two words.
const NEGATIONS: [&str; 3] = ["not", "never", "no"];

/// Words that make a claim a condition when they open its clause or come one or two words
/// before it.
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

/// The words of `claim` in `words`, joined, when they are there, not negated and not a
/// condition.
fn made(words: &[String], claim: &[Forms]) -> Option<String> {
    words
        .windows(claim.len())
        .enumerate()
        .find(|(start, window)| {
            window
                .iter()
                .zip(claim)
                .all(|(word, forms)| fills(word, forms))
                && !negated(words, *start)
                && !conditional(words, *start)
        })
        .map(|(_, window)| window.join(" "))
}

/// True when `word` is one of `forms`, or, for [`SUITES`], when the last segment of a
/// hyphenated `word` is (`unit-tests`, `smoke-checks`). Only suites look inside a hyphenated
/// word: `half-done` is not `done`.
fn fills(word: &str, forms: Forms) -> bool {
    forms.contains(&word)
        || (forms == SUITES
            && word
                .rsplit_once('-')
                .is_some_and(|(_, last)| forms.contains(&last)))
}

/// True when a conditional governs the claim at `start`: the clause opens with one ("When the
/// build passes"), or one is among the two words before the claim ("I'll merge once checks
/// pass"). A conditional further back belongs to a phrase of its own ("The crash when saving is
/// fixed").
fn conditional(words: &[String], start: usize) -> bool {
    let from = start.saturating_sub(2);
    words.first().is_some_and(|word| is_conditional(word))
        || words
            .get(from..start)
            .is_some_and(|before| before.iter().any(|word| is_conditional(word)))
}

/// True when `word` is one of [`CONDITIONALS`].
fn is_conditional(word: &str) -> bool {
    CONDITIONALS.contains(&word)
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
mod tests;
