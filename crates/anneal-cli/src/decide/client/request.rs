//! The question as Workers AI receives it, and the check that its options fit.

use std::collections::BTreeSet;

use serde_json::json;

use super::{Ask, AskError, MAX_OPTIONS};
use crate::decide::answer::NOUL_OPTIONS;
use crate::decide::id;
use crate::decide::point::Question;
use crate::decide::thresholds::Rule;

/// A choice has 2 to [`MAX_OPTIONS`] distinct options, each 1 to 64 of `[A-Za-z0-9_.:-]`
/// (the outcome-label alphabet, which JSON never escapes, so a row's size is bounded); other
/// questions have none.
pub(super) fn check_options(question: Question, options: &[String]) -> Result<(), AskError> {
    let problem = match question {
        Question::Noul | Question::Score => {
            (!options.is_empty()).then_some("only a choice question takes options")
        }
        Question::Choice => choice_problem(options),
    };
    problem.map_or(Ok(()), |reason| Err(AskError::Options { reason }))
}

/// What is wrong with a choice's options, if anything.
fn choice_problem(options: &[String]) -> Option<&'static str> {
    if !(2..=MAX_OPTIONS).contains(&options.len()) {
        return Some("a choice needs 2 to 32 options");
    }
    if !options.iter().all(|option| id::is_word(option)) {
        return Some("an option must be 1 to 64 of A-Z a-z 0-9 _ . : -");
    }
    let distinct: BTreeSet<&String> = options.iter().collect();
    (distinct.len() != options.len()).then_some("options must be distinct")
}

/// The Workers AI request: the question as the system message, the state as the user one.
pub(super) fn body(ask: &Ask<'_>, rule: &Rule) -> Result<Vec<u8>, AskError> {
    let options: Vec<&str> = match ask.question {
        Question::Noul => NOUL_OPTIONS.to_vec(),
        Question::Choice => ask.options.iter().map(String::as_str).collect(),
        Question::Score => Vec::new(),
    };
    let question = json!({
        "point": ask.point,
        "question": ask.question,
        "subject": rule.subject(),
        "options": options,
    });
    let body = json!({"messages": [
        {"role": "system", "content": question.to_string()},
        {"role": "user", "content": ask.state.as_str()},
    ]});
    serde_json::to_vec(&body).map_err(AskError::Encode)
}
