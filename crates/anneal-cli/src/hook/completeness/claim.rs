//! Success claims in a reply: the words that say the work is finished or the tests pass.
//!
//! A claim is one of [`CLAIMS`], a short sequence of slots, each filled by one word in one of
//! its listed forms (`pass`, `passes`, `passed`, `passing`), in one clause, in any letter case,
//! outside fenced code blocks, code spans and HTML comments (quoted output is not a claim).
//! Between two slots at most [`MAX_FILLERS`] filler words may stand: adverbs ([`FILLERS`]: "The
//! tests now pass", "Everything is still working") anywhere, and `the` or `both` before a suite
//! only ("All the tests pass", but not "Test the pass rate"). A suite slot takes every further
//! suite joined by `and` or `or` ("Tests and lint and builds pass"); the chain is walked from its
//! head only. A hyphenated suite word is a suite (`unit-tests`), and a hyphenated word whose
//! segments spell a whole claim of two or more words is that claim (`all-green`,
//! `ready-to-merge`); otherwise a hyphenated word is one word, so `half-done` is not `done`.
//!
//! A claim is withdrawn by structural rules within its clause: a negation (`not`, `never`,
//! `no`, or a word ending in `n't`) one or two words before it; a conditional's scope covering
//! its first word ("I'll merge once the CI checks pass"); for a claim led by its subject (`it`,
//! `everything`), a purpose word ([`PURPOSES`]) one or two words before it ("make sure
//! everything works"); and a question mark ending the clause ("Does it work?"). A conditional
//! ([`CONDITIONALS`]; not the adverb "once more", "once again" or a clause-final `once`, and
//! `after` or `before` only when they do not open the clause) opens a scope that runs to the
//! next `and`, `then` or `now`, to a second copula ("The panic when the file is empty is
//! resolved"), through a gerund right after the conditional ("The crash when saving is fixed")
//! or a past-tense verb not after a determiner ("Once the fix landed all tests pass"), or to the
//! clause end. The scopes of a clause are found in one forward pass, so with the bounded
//! fillers and head-only conjunct chains each claim is matched in time linear in the clause.
//! The scope is a heuristic: "Merge when lint is clean and all tests pass" claims, and so does
//! "Ship it when morning checks pass" (`morning` reads as a gerund).
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

/// Adverbs that may stand between two slots of a claim without changing what it says.
const FILLERS: [&str; 11] = [
    "still", "now", "all", "also", "finally", "really", "fully", "again", "just", "actually",
    "indeed",
];

/// Words that may stand before a suite slot only ("All the tests pass"), so "Test the pass
/// rate" is no claim.
const SUITE_FILLERS: [&str; 2] = ["the", "both"];

/// The most filler words between two slots.
const MAX_FILLERS: usize = 2;

/// Words that join one suite to the next.
const CONJUNCTIONS: [&str; 2] = ["and", "or"];

/// Words that negate a claim that follows within two words.
const NEGATIONS: [&str; 3] = ["not", "never", "no"];

/// Words that open a condition's scope ("once" and "after"/"before" with the exceptions in
/// [`opens_a_condition`]).
const CONDITIONALS: [&str; 8] = [
    "if", "once", "when", "whenever", "unless", "until", "after", "before",
];

/// Words after `once` that make it an adverb ("once more"), not a conditional.
const ONCE_ADVERBIAL: [&str; 2] = ["more", "again"];

/// Words that end a condition's scope: the main clause goes on after them.
const SCOPE_ENDS: [&str; 3] = ["and", "then", "now"];

/// Copulas: the second one after a conditional ends its scope and starts the main clause.
const COPULAS: [&str; 4] = ["is", "are", "was", "were"];

/// Words before an `-ed` word that make it an adjective ("the updated tests"), not the verb that
/// finishes a condition ("the fix landed").
const DETERMINERS: [&str; 14] = [
    "the", "a", "an", "all", "any", "some", "every", "this", "that", "these", "those", "its",
    "our", "their",
];

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

/// One clause: its words, and for each word whether a condition's scope covers it.
struct Clause {
    /// The lowercase words.
    words: Vec<String>,
    /// `governed[i]` is true when word `i` is inside a condition's scope.
    governed: Vec<bool>,
}

impl Clause {
    /// The clause of `words`, its scopes found in one pass.
    fn new(words: Vec<String>) -> Self {
        let governed = governed(&words);
        Self { words, governed }
    }
}

/// The first of [`CLAIMS`] that `reply` makes, as written there (lowercase), if any.
pub(crate) fn success_claim(reply: &str) -> Option<String> {
    let document = Document::parse(reply);
    let clauses: Vec<Clause> = document
        .lines()
        .iter()
        .flat_map(|line| clauses(&without_code_spans(&line.text)))
        .map(Clause::new)
        .collect();
    CLAIMS
        .into_iter()
        .find_map(|claim| clauses.iter().find_map(|clause| made(clause, claim)))
}

/// The words of `claim` in `clause`, joined, at the first place they are made and not withdrawn.
fn made(clause: &Clause, claim: &[Slot]) -> Option<String> {
    let words = clause.words.as_slice();
    let led_by_suite = matches!(claim.first(), Some(Suite));
    (0..words.len()).find_map(|start| {
        // A suite joined to an earlier one is judged with it: "Not all tests and lint pass".
        if led_by_suite && joins_a_suite(words, start) {
            return None;
        }
        let end = spelled(words, start, claim).or_else(|| span(words, start, claim))?;
        let led_by_subject = matches!(claim.first(), Some(Subject(_)));
        let withdrawn = negated(words, start)
            || clause.governed.get(start).is_some_and(|governed| *governed)
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
/// fills `slot`. [`SUITE_FILLERS`] are skipped only before a suite slot.
fn past_fillers(words: &[String], mut at: usize, slot: Slot) -> usize {
    let before_a_suite = matches!(slot, Suite);
    for _ in 0..MAX_FILLERS {
        let skip = words.get(at).is_some_and(|word| {
            let word = word.as_str();
            !slot.fills(word)
                && (FILLERS.contains(&word) || (before_a_suite && SUITE_FILLERS.contains(&word)))
        });
        if !skip {
            break;
        }
        at += 1;
    }
    at
}

/// The position after every `and <suite>` or `or <suite>` pair from `at`. The chain is walked
/// from its head only ([`joins_a_suite`] skips the other starts), so each word is passed once
/// per claim.
fn past_conjuncts(words: &[String], mut at: usize) -> usize {
    while words
        .get(at)
        .is_some_and(|word| CONJUNCTIONS.contains(&word.as_str()))
        && words.get(at + 1).is_some_and(|word| Suite.fills(word))
    {
        at += 2;
    }
    at
}

/// A condition's scope while it is open.
#[derive(Clone, Copy)]
struct Scope {
    /// True until the scope covers its first word.
    fresh: bool,
    /// True once the scope covers a copula.
    copula: bool,
}

impl Scope {
    /// The scope a conditional opens.
    const OPENED: Self = Self {
        fresh: true,
        copula: false,
    };
}

/// For each of `words`, whether a condition's scope covers it, in one forward pass.
///
/// A conditional ([`opens_a_condition`]) opens a scope, which covers the words after it until
/// one of [`SCOPE_ENDS`] or a second copula (the main clause's verb: "The panic when the file
/// is empty is resolved"), and through a gerund right after the conditional (a reduced clause:
/// "The crash when saving is fixed") or a past-tense verb (the condition's own verb: "Once the
/// fix landed all tests pass"), or the clause end.
fn governed(words: &[String]) -> Vec<bool> {
    let mut scope = None;
    let mut previous = None;
    words
        .iter()
        .enumerate()
        .map(|(at, word)| {
            let word = word.as_str();
            let next = words.get(at + 1).map(String::as_str);
            let (covered, after) = if opens_a_condition(word, at, next) {
                (false, Some(Scope::OPENED))
            } else {
                scope.map_or((false, None), |open| within(open, word, previous))
            };
            scope = after;
            previous = Some(word);
            covered
        })
        .collect()
}

/// Whether the open `scope` covers `word` (after `previous`), and the scope after it.
fn within(scope: Scope, word: &str, previous: Option<&str>) -> (bool, Option<Scope>) {
    let copula = COPULAS.contains(&word);
    if SCOPE_ENDS.contains(&word) || (copula && scope.copula) {
        return (false, None);
    }
    let reduced = scope.fresh && is_gerund(word);
    let finished =
        is_past(word) && !previous.is_some_and(|previous| DETERMINERS.contains(&previous));
    let after = Scope {
        fresh: false,
        copula: scope.copula || copula,
    };
    (true, (!reduced && !finished).then_some(after))
}

/// True when `word`, at `at` in its clause and followed by `next`, opens a condition: one of
/// [`CONDITIONALS`], except an adverbial `once` ("once more", "once again", or at the clause
/// end) and a clause-opening `after` or `before` ("After the rebase all tests pass").
fn opens_a_condition(word: &str, at: usize, next: Option<&str>) -> bool {
    match word {
        "once" => next.is_some_and(|next| !ONCE_ADVERBIAL.contains(&next)),
        "after" | "before" => at > 0,
        _ => CONDITIONALS.contains(&word),
    }
}

/// True when `word` reads as a gerund (`saving`, `running`), not `everything` or `nothing`.
fn is_gerund(word: &str) -> bool {
    word.len() > 4 && word.ends_with("ing") && !word.ends_with("thing")
}

/// True when `word` reads as a past tense (`landed`, `bumped`), not `need` or `speed`.
fn is_past(word: &str) -> bool {
    word.len() > 3 && word.ends_with("ed") && !word.ends_with("eed")
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
