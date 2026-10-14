//! Success claims in a reply: the words that say the work is finished or the tests pass.
//!
//! A claim is one of [`CLAIMS`], a short sequence of slots, each filled by one word in one of
//! its listed forms (`pass`, `passes`, `passed`, `passing`), in one clause, in any letter case,
//! outside fenced code blocks, code spans and HTML comments (quoted output is not a claim).
//! Between two slots at most [`MAX_FILLERS`] filler words ([`FILLERS`]: "The tests now pass",
//! "Everything is still working") may stand, and a suite slot takes up to [`MAX_CONJUNCTS`]
//! more suites joined by `and` or `or` ("Tests and lint pass"), so each claim is matched in a
//! bounded number of steps per word. A hyphenated suite word is a suite (`unit-tests`), and a
//! hyphenated word whose segments spell a whole claim of two or more words is that claim
//! (`all-green`, `ready-to-merge`); otherwise a hyphenated word is one word, so `half-done` is
//! not `done`.
//!
//! A claim is withdrawn by structural rules within its clause: a negation (`not`, `never`,
//! `no`, or a word ending in `n't`) one or two words before it; a conditional
//! ([`CONDITIONALS`]) anywhere before it ("I'll merge once the CI checks pass"), unless the
//! conditional is the adverb "once more" or "once again", heads a reduced clause (a gerund
//! right after it: "The crash when saving is fixed"), or its own clause has ended (two copulas
//! between it and the claim: "The panic when the file is empty is resolved"); for a claim led
//! by its subject (`it`, `everything`), a purpose word ([`PURPOSES`]) one or two words before
//! it ("make sure everything works"); and a question mark ending the clause ("Does it work?").
//! Clauses end at `.`, `!`, `?`, `;`, `:`, `,`, a dash, a bracket, a line end and the words
//! `so` and `but`, so in "No failures. Done." the "No" does not reach "Done", and in "Done
//! (anything else?)" or "All tests pass so should I merge?" only the question is withdrawn. A
//! hyphen ends a clause only when it is not between two letters or digits: `hard - fixed` and
//! `hard--fixed` are two clauses. False positives ("done reading the file", "how it works",
//! "Did the tests pass, or should I re-run?") are expected; the hook's block cap bounds what
//! they cost.

use crate::lint::spec::markdown::{Document, without_code_spans};

use Slot::{Subject, Suite, Word};

/// The forms one word of a claim may take, lowercase.
type Forms = &'static [&'static str];

/// One position of a claim.
#[derive(Clone, Copy, Debug)]
enum Slot {
    /// A word in one of these forms.
    Word(Forms),
    /// The claim's subject: purpose words before it withdraw the claim.
    Subject(Forms),
    /// What passes: one of [`SUITES`], or a hyphenated word ending in one.
    Suite,
}

impl Slot {
    /// True when `word` fills this slot.
    fn fills(self, word: &str) -> bool {
        match self {
            Word(forms) | Subject(forms) => forms.contains(&word),
            Suite => {
                SUITES.contains(&word)
                    || word
                        .rsplit_once('-')
                        .is_some_and(|(_, last)| SUITES.contains(&last))
            }
        }
    }
}

/// What passes: tests, checks, builds or lint.
const SUITES: Forms = &[
    "tests", "test", "checks", "check", "builds", "build", "lint", "lints",
];

/// `pass` in every form, after a suite.
const PASS: Slot = Word(&["pass", "passes", "passed", "passing"]);

/// Who works in a "works" claim.
const WORKER: Slot = Subject(&["everything", "it"]);

/// Who works in a "works" claim, contractions included.
const WORKER_OR_CONTRACTION: Slot = Subject(&["everything", "it", "everything's", "it's"]);

/// `works` in the forms a "works" claim takes.
const WORKS: Slot = Word(&["works", "working"]);

/// The claims in report order: the first one a reply makes is reported. The phrases from #26
/// come first, longest first, and the #84 additions follow, so a reply making a #26 claim
/// still reports that claim (unless a structural rule now withdraws it).
const CLAIMS: [&[Slot]; 17] = [
    &[Word(&["all"]), Suite, PASS],
    &[Suite, PASS],
    &[Word(&["works", "work"]), Word(&["now"])],
    &[Word(&["all"]), Word(&["green"])],
    &[Word(&["done"])],
    // Bare `pass` is left out: "pass the flag" is not a claim.
    &[Word(&["passes", "passed", "passing"])],
    &[Word(&["green"])],
    &[Word(&["fixed"])],
    &[Word(&["complete", "completed"])],
    &[Word(&["finished"])],
    &[Word(&["succeeded", "succeeds"])],
    &[Word(&["ready"]), Word(&["to"]), Word(&["merge"])],
    &[Word(&["ready"]), Word(&["for"]), Word(&["review"])],
    &[WORKER, Word(&["is"]), Word(&["working"])],
    &[WORKER_OR_CONTRACTION, WORKS],
    &[Word(&["implemented"])],
    &[Word(&["resolved"])],
];

/// Words that may stand between two slots of a claim without changing what it says.
const FILLERS: [&str; 13] = [
    "still", "now", "all", "also", "finally", "really", "fully", "the", "both", "again", "just",
    "actually", "indeed",
];

/// The most filler words between two slots.
const MAX_FILLERS: usize = 2;

/// Words that join one suite to the next.
const CONJUNCTIONS: [&str; 2] = ["and", "or"];

/// The most suites joined to the first.
const MAX_CONJUNCTS: usize = 2;

/// Words that negate a claim that follows within two words.
const NEGATIONS: [&str; 3] = ["not", "never", "no"];

/// Words that make the rest of their clause a condition, not a claim.
const CONDITIONALS: [&str; 8] = [
    "if", "once", "when", "whenever", "unless", "until", "after", "before",
];

/// Words after `once` that make it an adverb ("once more"), not a conditional.
const ONCE_ADVERBIAL: [&str; 2] = ["more", "again"];

/// Copulas: two of them after a conditional mean its clause ended and the main one began.
const COPULAS: [&str; 4] = ["is", "are", "was", "were"];

/// Words that make a "works" claim a goal, not a report, one or two words before its subject.
const PURPOSES: [&str; 9] = [
    "sure", "ensure", "verify", "check", "whether", "get", "keep", "need", "to",
];

/// Characters that end a clause.
const CLAUSE_ENDS: [char; 13] = [
    '.', '!', '?', ';', ':', ',', '-', '\u{2013}', '\u{2014}', '(', ')', '[', ']',
];

/// Words that end a clause and start another.
const CLAUSE_WORDS: [&str; 2] = ["so", "but"];

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

/// The words of `claim` in `words`, joined, at the first place they are made and not withdrawn.
fn made(words: &[String], claim: &[Slot]) -> Option<String> {
    let led_by_suite = matches!(claim.first(), Some(Suite));
    (0..words.len()).find_map(|start| {
        // A suite joined to an earlier one is judged with it: "Not all tests and lint pass".
        if led_by_suite && joins_a_suite(words, start) {
            return None;
        }
        let end = spelled(words, start, claim).or_else(|| span(words, start, claim))?;
        let led_by_subject = matches!(claim.first(), Some(Subject(_)));
        let withdrawn = negated(words, start)
            || conditional(words, start)
            || (led_by_subject && purposed(words, start));
        if withdrawn {
            None
        } else {
            words.get(start..end).map(|made| made.join(" "))
        }
    })
}

/// The end of `claim` when the word at `start` spells all of it with hyphens (`all-green`).
fn spelled(words: &[String], start: usize, claim: &[Slot]) -> Option<usize> {
    let word = words.get(start)?;
    let whole = claim.len() > 1
        && word.split('-').count() == claim.len()
        && word
            .split('-')
            .zip(claim)
            .all(|(segment, slot)| slot.fills(segment));
    whole.then_some(start + 1)
}

/// The end of `claim` when its slots are filled from `start` on, with fillers and conjuncts.
fn span(words: &[String], start: usize, claim: &[Slot]) -> Option<usize> {
    let mut at = start;
    for (index, slot) in claim.iter().enumerate() {
        if index > 0 {
            at = past_fillers(words, at, *slot);
        }
        if !slot.fills(words.get(at)?) {
            return None;
        }
        at += 1;
        if matches!(slot, Suite) {
            at = past_conjuncts(words, at);
        }
    }
    Some(at)
}

/// The position after at most [`MAX_FILLERS`] filler words from `at`, stopping at a word that
/// fills `slot`.
fn past_fillers(words: &[String], mut at: usize, slot: Slot) -> usize {
    for _ in 0..MAX_FILLERS {
        let skip = words
            .get(at)
            .is_some_and(|word| !slot.fills(word) && FILLERS.contains(&word.as_str()));
        if !skip {
            break;
        }
        at += 1;
    }
    at
}

/// The position after at most [`MAX_CONJUNCTS`] `and <suite>` or `or <suite>` pairs from `at`.
fn past_conjuncts(words: &[String], mut at: usize) -> usize {
    for _ in 0..MAX_CONJUNCTS {
        let joined = words
            .get(at)
            .is_some_and(|word| CONJUNCTIONS.contains(&word.as_str()))
            && words.get(at + 1).is_some_and(|word| Suite.fills(word));
        if !joined {
            break;
        }
        at += 2;
    }
    at
}

/// True when a conditional before `start` governs the claim there.
fn conditional(words: &[String], start: usize) -> bool {
    words.get(..start).is_some_and(|before| {
        before
            .iter()
            .enumerate()
            .any(|(at, word)| CONDITIONALS.contains(&word.as_str()) && governs(words, at, start))
    })
}

/// True when the conditional at `at` governs the claim at `start`: it is no adverbial `once`,
/// heads no reduced clause before the claim, and its own clause has not ended (fewer than two
/// copulas between them).
fn governs(words: &[String], at: usize, start: usize) -> bool {
    let next = words.get(at + 1).map(String::as_str);
    let adverbial = words.get(at).is_some_and(|word| word == "once")
        && next.is_some_and(|next| ONCE_ADVERBIAL.contains(&next));
    let reduced = at + 1 < start && next.is_some_and(is_gerund);
    let copulas = words.get(at + 1..start).map_or(0, |between| {
        between
            .iter()
            .filter(|word| COPULAS.contains(&word.as_str()))
            .count()
    });
    !adverbial && !reduced && copulas < 2
}

/// True when `word` reads as a gerund (`saving`, `running`), not `everything` or `nothing`.
fn is_gerund(word: &str) -> bool {
    word.len() > 4 && word.ends_with("ing") && !word.ends_with("thing")
}

/// True when one of the two words before `start` is a negation.
fn negated(words: &[String], start: usize) -> bool {
    two_before(words, start).any(|word| NEGATIONS.contains(&word.as_str()) || word.ends_with("n't"))
}

/// True when one of the two words before `start` is a purpose word.
fn purposed(words: &[String], start: usize) -> bool {
    two_before(words, start).any(|word| PURPOSES.contains(&word.as_str()))
}

/// True when the word at `start` follows a suite and a conjunction (`tests and lint`).
fn joins_a_suite(words: &[String], start: usize) -> bool {
    let mut before = two_before(words, start);
    let suite = before.next().is_some_and(|word| Suite.fills(word));
    let joined = before
        .next()
        .is_some_and(|word| CONJUNCTIONS.contains(&word.as_str()));
    suite && joined
}

/// The (at most) two words before `start`, in order.
fn two_before(words: &[String], start: usize) -> impl Iterator<Item = &String> {
    words.iter().take(start).skip(start.saturating_sub(2))
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
            end_word(&mut word, &mut words, &mut clauses);
            if CLAUSE_ENDS.contains(&c) {
                let clause = std::mem::take(&mut words);
                if !clause.is_empty() && c != '?' {
                    clauses.push(clause);
                }
            }
        }
    }
    end_word(&mut word, &mut words, &mut clauses);
    if !words.is_empty() {
        clauses.push(words);
    }
    clauses
}

/// Moves the finished `word` into `words`, or, for a [`CLAUSE_WORDS`] word, ends the clause.
fn end_word(word: &mut String, words: &mut Vec<String>, clauses: &mut Vec<Vec<String>>) {
    if word.is_empty() {
        return;
    }
    let word = finished(std::mem::take(word));
    if CLAUSE_WORDS.contains(&word.as_str()) {
        let clause = std::mem::take(words);
        if !clause.is_empty() {
            clauses.push(clause);
        }
    } else {
        words.push(word);
    }
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
