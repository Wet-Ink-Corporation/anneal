//! EARS acceptance criteria: a deterministic parser for the Easy Approach to Requirements
//! Syntax, with the grammar `docs/spec.md` documents.
//!
//! ```text
//! criterion := [WHERE text] [WHILE text] [WHEN text | IF text THEN] THE system SHALL response
//! ```
//!
//! Keywords are whole words in capitals (a trailing `,`, `.`, `:` or `;` is ignored), so a
//! lowercase `when` or `the` is ordinary text. Inside a clause the keywords are reserved: a
//! clause runs up to the next one. Commas between clauses are optional.

use std::fmt::{self, Display, Formatter};

/// A precondition clause. The variant order is the order EARS allows them in (`WHEN` and `IF`
/// share the last place, and only one of them may appear).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Clause {
    /// `WHERE <feature>`: an optional feature.
    Where,
    /// `WHILE <state>`: a state the system is in.
    While,
    /// `WHEN <trigger>`: an event.
    When,
    /// `IF <condition>, THEN`: unwanted behaviour.
    If,
}

impl Clause {
    /// The keyword, in capitals.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Where => "WHERE",
            Self::While => "WHILE",
            Self::When => "WHEN",
            Self::If => "IF",
        }
    }

    /// Its place in EARS order: `WHERE`, then `WHILE`, then `WHEN` or `IF`.
    const fn rank(self) -> u8 {
        match self {
            Self::Where => 0,
            Self::While => 1,
            Self::When | Self::If => 2,
        }
    }

    /// The clause in a template, with its trailing comma (and `THEN` for `IF`).
    const fn placeholder(self) -> &'static str {
        match self {
            Self::Where => "WHERE <feature>, ",
            Self::While => "WHILE <state>, ",
            Self::When => "WHEN <trigger>, ",
            Self::If => "IF <condition>, THEN ",
        }
    }

    /// The clause `word` names in any letter case, ignoring trailing punctuation.
    fn any_case(word: &str) -> Option<Self> {
        let word = trim_punctuation(word);
        [Self::Where, Self::While, Self::When, Self::If]
            .into_iter()
            .find(|clause| clause.keyword().eq_ignore_ascii_case(word))
    }
}

impl Display for Clause {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// A criterion that parsed: its precondition clauses, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// The clauses before `THE <system> SHALL`; empty for a ubiquitous requirement.
    pub clauses: Vec<Clause>,
}

impl Pattern {
    /// The EARS name of the pattern: `ubiquitous`, `event-driven`, `state-driven`,
    /// `unwanted-behaviour`, `optional-feature`, or `complex` for more than one clause.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self.clauses.as_slice() {
            [] => "ubiquitous",
            [Clause::When] => "event-driven",
            [Clause::While] => "state-driven",
            [Clause::If] => "unwanted-behaviour",
            [Clause::Where] => "optional-feature",
            _ => "complex",
        }
    }
}

/// Why a criterion is not EARS. The message is one line, for a finding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EarsError {
    /// Nothing but whitespace.
    #[error("the criterion is empty")]
    Empty,
    /// The first word is not a keyword (keywords are in capitals).
    #[error(
        "starts with `{word}`, not an EARS keyword (`WHERE`, `WHILE`, `WHEN`, `IF` or `THE`, in capitals)"
    )]
    UnknownStart {
        /// The first word, as written.
        word: String,
    },
    /// A clause comes after one EARS puts later.
    #[error("`{clause}` comes after `{after}`; EARS orders clauses WHERE, WHILE, then WHEN or IF")]
    OutOfOrder {
        /// The misplaced clause.
        clause: Clause,
        /// The clause it follows.
        after: Clause,
    },
    /// A clause appears twice.
    #[error("`{0}` appears twice")]
    Repeated(Clause),
    /// Both `WHEN` and `IF`.
    #[error("has both `WHEN` and `IF`; a criterion has one trigger")]
    TwoTriggers,
    /// A clause keyword followed directly by another keyword.
    #[error("`{0}` has no text before the next keyword")]
    EmptyClause(Clause),
    /// `IF <condition>` not followed by `THEN`.
    #[error("`IF` has no `THEN` before `THE`")]
    MissingThen,
    /// `THEN` with no `IF` before it.
    #[error("`THEN` without `IF`")]
    StrayThen,
    /// After a keyword, a word other than the one the grammar needs next (such as a
    /// lowercase `the` after `THEN`).
    #[error("expected `{expected}` after `{after}`, found `{found}`")]
    Expected {
        /// The last keyword read.
        after: &'static str,
        /// What had to come next.
        expected: &'static str,
        /// The word found, as written.
        found: String,
    },
    /// `SHALL` reached without `THE <system>` before it.
    #[error("`SHALL` is not preceded by `THE <system>`")]
    MissingThe,
    /// `THE` directly followed by `SHALL`.
    #[error("`THE` names no system before `SHALL`")]
    MissingSystem,
    /// No `SHALL`.
    #[error("has no `SHALL`")]
    MissingShall,
    /// Nothing after `SHALL`.
    #[error("`SHALL` is followed by no response")]
    EmptyResponse,
}

/// What a word is to the grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Word {
    Clause(Clause),
    Then,
    The,
    Shall,
    Text,
}

impl Word {
    fn of(word: &str) -> Self {
        match trim_punctuation(word) {
            "WHERE" => Self::Clause(Clause::Where),
            "WHILE" => Self::Clause(Clause::While),
            "WHEN" => Self::Clause(Clause::When),
            "IF" => Self::Clause(Clause::If),
            "THEN" => Self::Then,
            "THE" => Self::The,
            "SHALL" => Self::Shall,
            _ => Self::Text,
        }
    }
}

/// Parses one acceptance criterion.
///
/// ```
/// use anneal_cli::lint::spec::ears::{Clause, EarsError, parse};
///
/// let pattern = parse("WHILE offline, WHEN a file changes, THE app SHALL queue it.")?;
/// assert_eq!(pattern.clauses, [Clause::While, Clause::When]);
/// assert_eq!(parse("The app SHALL queue it."), Err(EarsError::UnknownStart { word: "The".to_owned() }));
/// # Ok::<(), EarsError>(())
/// ```
///
/// # Errors
/// The first [`EarsError`] met reading left to right.
pub fn parse(text: &str) -> Result<Pattern, EarsError> {
    let words: Vec<(&str, Word)> = text
        .split_whitespace()
        .map(|word| (word, Word::of(word)))
        .collect();
    let mut rest = words.as_slice();
    let mut clauses: Vec<Clause> = Vec::new();
    loop {
        match rest {
            [] if clauses.is_empty() => return Err(EarsError::Empty),
            [] => return Err(EarsError::MissingShall),
            [(_, Word::Clause(clause)), tail @ ..] => {
                admit(&clauses, *clause)?;
                let text_words = tail
                    .iter()
                    .take_while(|(_, word)| *word == Word::Text)
                    .count();
                if text_words == 0 {
                    return Err(EarsError::EmptyClause(*clause));
                }
                clauses.push(*clause);
                // `text_words` counts a prefix of `tail`, so the range is always in bounds.
                rest = tail.get(text_words..).unwrap_or_default();
                if *clause == Clause::If {
                    let [(_, Word::Then), after_then @ ..] = rest else {
                        return Err(EarsError::MissingThen);
                    };
                    rest = after_then;
                }
            }
            [(_, Word::Then), ..] => return Err(EarsError::StrayThen),
            [(_, Word::The), tail @ ..] => {
                return main_clause(tail).map(|()| Pattern { clauses });
            }
            [(_, Word::Shall), ..] if !clauses.is_empty() => return Err(EarsError::MissingThe),
            [(word, Word::Text), ..] if let Some(last) = clauses.last() => {
                // A clause's text runs to the next keyword, so text here follows `THEN`.
                return Err(EarsError::Expected {
                    after: if *last == Clause::If {
                        "THEN"
                    } else {
                        last.keyword()
                    },
                    expected: "THE <system>",
                    found: (*word).to_owned(),
                });
            }
            [(word, Word::Shall | Word::Text), ..] => {
                return Err(EarsError::UnknownStart {
                    word: (*word).to_owned(),
                });
            }
        }
    }
}

/// Checks that `clause` may follow `seen`.
fn admit(seen: &[Clause], clause: Clause) -> Result<(), EarsError> {
    if seen.contains(&clause) {
        return Err(EarsError::Repeated(clause));
    }
    let other_trigger = match clause {
        Clause::When => Some(Clause::If),
        Clause::If => Some(Clause::When),
        Clause::Where | Clause::While => None,
    };
    if other_trigger.is_some_and(|other| seen.contains(&other)) {
        return Err(EarsError::TwoTriggers);
    }
    match seen.last() {
        Some(&after) if after.rank() > clause.rank() => {
            Err(EarsError::OutOfOrder { clause, after })
        }
        _ => Ok(()),
    }
}

/// `<system> SHALL <response>`, after `THE`.
fn main_clause(words: &[(&str, Word)]) -> Result<(), EarsError> {
    let Some(shall) = words.iter().position(|(_, word)| *word == Word::Shall) else {
        return Err(EarsError::MissingShall);
    };
    if shall == 0 {
        return Err(EarsError::MissingSystem);
    }
    if words.len() <= shall.saturating_add(1) {
        return Err(EarsError::EmptyResponse);
    }
    Ok(())
}

/// The EARS template closest to `text`: the clauses whose keywords (in any letter case) start
/// the text or follow a comma, in EARS order and at most one trigger, then
/// `THE <system> SHALL <response>.`
#[must_use]
pub fn template(text: &str) -> String {
    let mut clauses: Vec<Clause> = Vec::new();
    let mut at_clause_start = true;
    for word in text.split_whitespace() {
        if at_clause_start
            && let Some(clause) = Clause::any_case(word)
            && admit(&clauses, clause).is_ok()
        {
            clauses.push(clause);
        }
        at_clause_start = word.ends_with(',');
    }
    let mut out: String = clauses.iter().map(|clause| clause.placeholder()).collect();
    out.push_str("THE <system> SHALL <response>.");
    out
}

/// `word` without trailing `,`, `.`, `:` or `;`.
fn trim_punctuation(word: &str) -> &str {
    word.trim_end_matches([',', '.', ':', ';'])
}

#[cfg(test)]
mod tests;
