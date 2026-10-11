//! Reading Clef's answer out of a Workers AI response.
//!
//! Workers AI wraps a model's output as `{"result": {"response": ...}, "success": ...}`. The
//! response is Clef's JSON, as text or as an object: `{"probabilities": {<option>: p}}` for
//! `noul` (options `yes` and `no`) and `choice`, `{"score": v}` for `score`. The reader is
//! tolerant: unknown fields (Clef's `confidence` among them) are ignored, and only the
//! probabilities of the options asked are kept. A missing or out-of-range number is an error,
//! and so are noul (when `no` is given) or choice probabilities that do not sum to 1 within
//! [`SUM_TOLERANCE`].

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::decide::point::Question;

/// The options of a `noul` question.
pub const NOUL_OPTIONS: [&str; 2] = ["yes", "no"];

/// Why a response holds no usable answer; the client reports it as a bad response.
#[derive(Debug, thiserror::Error)]
pub enum AnswerError {
    /// The body is not JSON.
    #[error("the response is not JSON")]
    Json(#[source] serde_json::Error),
    /// Workers AI reported `success: false`.
    #[error("Workers AI reported success: false")]
    Unsuccessful,
    /// The model's output is not a JSON object.
    #[error("the model's output is not a JSON object")]
    Shape,
    /// A probability or the score the question needs is missing or not a number.
    #[error("no probability for {name}")]
    Missing {
        /// The option, or `score`.
        name: String,
    },
    /// A probability or score is outside 0..=1.
    #[error("{name} is outside 0..=1")]
    OutOfRange {
        /// The option, or `score`.
        name: String,
    },
    /// The probabilities of the options asked sum to more than [`SUM_TOLERANCE`] away from 1.
    #[error("the probabilities sum to {sum}, not 1 within {SUM_TOLERANCE}")]
    Sum {
        /// Their sum.
        sum: f64,
    },
}

/// How far from 1 the probabilities of a noul or choice answer may sum.
pub const SUM_TOLERANCE: f64 = 0.05;

/// Clef's answer to one question.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    probability: f64,
    top: Option<String>,
    probabilities: BTreeMap<String, f64>,
}

impl Answer {
    /// The probability the thresholds apply to: `yes` for noul, the top option's for choice,
    /// the score for score.
    #[must_use]
    pub const fn probability(&self) -> f64 {
        self.probability
    }

    /// The top option of a choice; `None` for noul and score.
    #[must_use]
    pub fn top(&self) -> Option<&str> {
        self.top.as_deref()
    }

    /// The probability of each option asked (empty for score).
    #[must_use]
    pub const fn probabilities(&self) -> &BTreeMap<String, f64> {
        &self.probabilities
    }
}

/// The Workers AI wrapper; every field optional.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Wrapper {
    result: Option<Value>,
    success: Option<bool>,
}

/// Reads the answer to `question` (with `options` for a choice) from a response body.
///
/// # Errors
/// [`AnswerError`] when the body is not JSON, reports failure, holds no JSON object, or lacks
/// a number in 0..=1 the question needs.
pub fn parse(body: &[u8], question: Question, options: &[String]) -> Result<Answer, AnswerError> {
    let wrapper: Wrapper = serde_json::from_slice(body).map_err(AnswerError::Json)?;
    if wrapper.success == Some(false) {
        return Err(AnswerError::Unsuccessful);
    }
    let output = model_output(wrapper.result).ok_or(AnswerError::Shape)?;
    match question {
        Question::Noul => {
            let asked: Vec<String> = NOUL_OPTIONS.iter().map(|o| (*o).to_owned()).collect();
            let probabilities = probabilities_of(&output, &asked, &["yes"])?;
            // `no` may be left out, as 1 - `yes`; given, the two must add up.
            if probabilities.len() == NOUL_OPTIONS.len() {
                sums_to_one(&probabilities)?;
            }
            let probability = number(probabilities.get("yes"), "yes")?;
            Ok(Answer {
                probability,
                top: None,
                probabilities,
            })
        }
        Question::Choice => {
            let required: Vec<&str> = options.iter().map(String::as_str).collect();
            let probabilities = probabilities_of(&output, options, &required)?;
            sums_to_one(&probabilities)?;
            let (top, probability) = top_of(options, &probabilities)?;
            Ok(Answer {
                probability,
                top: Some(top),
                probabilities,
            })
        }
        Question::Score => Ok(Answer {
            probability: in_range(output.get("score").and_then(Value::as_f64), "score")?,
            top: None,
            probabilities: BTreeMap::new(),
        }),
    }
}

/// The model's output as an object: `result.response` as text or object, else `result`.
fn model_output(result: Option<Value>) -> Option<Map<String, Value>> {
    let Value::Object(mut result) = result? else {
        return None;
    };
    match result.remove("response") {
        Some(Value::String(text)) => match serde_json::from_str(&text) {
            Ok(Value::Object(output)) => Some(output),
            Ok(_) | Err(_) => None,
        },
        Some(Value::Object(output)) => Some(output),
        Some(_) => None,
        None => Some(result),
    }
}

/// The probabilities of the `asked` options present in `output`, every `required` one there.
fn probabilities_of(
    output: &Map<String, Value>,
    asked: &[String],
    required: &[&str],
) -> Result<BTreeMap<String, f64>, AnswerError> {
    let given = output.get("probabilities").and_then(Value::as_object);
    let mut kept = BTreeMap::new();
    for option in asked {
        let value = given
            .and_then(|given| given.get(option))
            .and_then(Value::as_f64);
        if value.is_none() && !required.contains(&option.as_str()) {
            continue;
        }
        // The map owns its keys; the options stay with the caller.
        kept.insert(option.clone(), in_range(value, option)?);
    }
    Ok(kept)
}

/// Checks that `probabilities` sum to 1 within [`SUM_TOLERANCE`].
fn sums_to_one(probabilities: &BTreeMap<String, f64>) -> Result<(), AnswerError> {
    let sum: f64 = probabilities.values().sum();
    if (sum - 1.0).abs() <= SUM_TOLERANCE {
        Ok(())
    } else {
        Err(AnswerError::Sum { sum })
    }
}

/// The first option with the highest probability.
fn top_of(
    options: &[String],
    probabilities: &BTreeMap<String, f64>,
) -> Result<(String, f64), AnswerError> {
    let mut best: Option<(&String, f64)> = None;
    for option in options {
        let probability = number(probabilities.get(option), option)?;
        if best.is_none_or(|(_, highest)| probability > highest) {
            best = Some((option, probability));
        }
    }
    // The answer owns its top option; the options stay with the caller.
    best.map(|(option, probability)| (option.clone(), probability))
        .ok_or_else(|| AnswerError::Missing {
            name: "any option".to_owned(),
        })
}

/// A kept probability, which [`probabilities_of`] has already range-checked.
fn number(value: Option<&f64>, name: &str) -> Result<f64, AnswerError> {
    value.copied().ok_or_else(|| AnswerError::Missing {
        name: name.to_owned(),
    })
}

/// `value` when it is a number within 0..=1.
fn in_range(value: Option<f64>, name: &str) -> Result<f64, AnswerError> {
    match value {
        None => Err(AnswerError::Missing {
            name: name.to_owned(),
        }),
        Some(number) if (0.0..=1.0).contains(&number) => Ok(number),
        Some(_) => Err(AnswerError::OutOfRange {
            name: name.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{Answer, AnswerError, parse};
    use crate::decide::point::Question;

    /// A Workers AI response whose model text is `inner` as a JSON string.
    fn workers_ai(inner: &Value) -> Vec<u8> {
        json!({"result": {"response": inner.to_string()}, "success": true, "errors": []})
            .to_string()
            .into_bytes()
    }

    fn options(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The probability is the number in the response, bit for bit (it is never computed).
    fn assert_probability(answer: &Answer, expected: f64) {
        assert_eq!(
            answer.probability().to_bits(),
            expected.to_bits(),
            "{} != {expected}",
            answer.probability()
        );
    }

    #[test]
    fn noul_takes_the_probability_of_yes_and_ignores_confidence() -> Result<(), AnswerError> {
        let body = workers_ai(&json!({
            "probabilities": {"yes": 0.2, "no": 0.8},
            "confidence": 0.99
        }));

        let answer = parse(&body, Question::Noul, &[])?;

        assert_probability(&answer, 0.2);
        assert_eq!(answer.top(), None);
        Ok(())
    }

    #[test]
    fn choice_takes_the_top_options_probability() -> Result<(), AnswerError> {
        let body = workers_ai(&json!({
            "probabilities": {"build": 0.7, "plan": 0.2, "ship": 0.1, "extra": 0.9}
        }));

        let answer = parse(
            &body,
            Question::Choice,
            &options(&["plan", "build", "ship"]),
        )?;

        assert_probability(&answer, 0.7);
        assert_eq!(answer.top(), Some("build"));
        let kept: Vec<&str> = answer.probabilities().keys().map(String::as_str).collect();
        assert_eq!(
            kept,
            ["build", "plan", "ship"],
            "only the options asked are kept"
        );
        Ok(())
    }

    #[test]
    fn a_tie_goes_to_the_first_option_given() -> Result<(), AnswerError> {
        let body = workers_ai(&json!({"probabilities": {"a": 0.5, "b": 0.5}}));

        let answer = parse(&body, Question::Choice, &options(&["b", "a"]))?;

        assert_eq!(answer.top(), Some("b"));
        Ok(())
    }

    #[test]
    fn score_takes_the_value() -> Result<(), AnswerError> {
        let body = workers_ai(&json!({"score": 0.75, "confidence": 0.1}));

        let answer = parse(&body, Question::Score, &[])?;

        assert_probability(&answer, 0.75);
        Ok(())
    }

    #[test]
    fn a_response_object_instead_of_text_is_read_too() -> Result<(), AnswerError> {
        let body = json!({"result": {"response": {"probabilities": {"yes": 0.9}}}})
            .to_string()
            .into_bytes();

        let answer: Answer = parse(&body, Question::Noul, &[])?;

        assert_probability(&answer, 0.9);
        Ok(())
    }

    #[test]
    fn a_missing_probability_is_a_bad_response() {
        let cases = [
            (
                json!({"probabilities": {"no": 0.9}}),
                Question::Noul,
                options(&[]),
            ),
            (
                json!({"probabilities": {"a": 0.9}}),
                Question::Choice,
                options(&["a", "b"]),
            ),
            (json!({"confidence": 0.9}), Question::Score, options(&[])),
        ];
        for (inner, question, asked) in cases {
            let result = parse(&workers_ai(&inner), question, &asked);
            assert!(
                matches!(result, Err(AnswerError::Missing { .. })),
                "{inner} for {question}: {result:?}"
            );
        }
    }

    #[test]
    fn a_probability_outside_zero_to_one_is_a_bad_response() {
        let body = workers_ai(&json!({"probabilities": {"yes": 1.5}}));

        assert!(matches!(
            parse(&body, Question::Noul, &[]),
            Err(AnswerError::OutOfRange { .. })
        ));
    }

    #[test]
    fn probabilities_that_do_not_sum_to_one_within_005_are_a_bad_response() {
        let cases = [
            (
                json!({"probabilities": {"yes": 0.9, "no": 0.3}}),
                Question::Noul,
                options(&[]),
            ),
            (
                json!({"probabilities": {"yes": 0.5, "no": 0.44}}),
                Question::Noul,
                options(&[]),
            ),
            (
                json!({"probabilities": {"a": 0.5, "b": 0.3, "c": 0.1}}),
                Question::Choice,
                options(&["a", "b", "c"]),
            ),
            (
                json!({"probabilities": {"a": 0.9, "b": 0.9}}),
                Question::Choice,
                options(&["a", "b"]),
            ),
        ];
        for (inner, question, asked) in cases {
            let result = parse(&workers_ai(&inner), question, &asked);
            assert!(
                matches!(result, Err(AnswerError::Sum { .. })),
                "{inner} for {question}: {result:?}"
            );
        }
    }

    #[test]
    fn probabilities_within_005_of_one_are_kept() -> Result<(), AnswerError> {
        let cases = [
            (
                json!({"probabilities": {"yes": 0.5, "no": 0.46}}),
                Question::Noul,
                options(&[]),
            ),
            (
                json!({"probabilities": {"yes": 0.97}}),
                Question::Noul,
                options(&[]),
            ),
            (
                json!({"probabilities": {"a": 0.6, "b": 0.44, "extra": 0.9}}),
                Question::Choice,
                options(&["a", "b"]),
            ),
        ];
        for (inner, question, asked) in cases {
            parse(&workers_ai(&inner), question, &asked)?;
        }
        Ok(())
    }

    #[test]
    fn an_unsuccessful_or_unreadable_response_is_a_bad_response() {
        let failed = json!({"result": null, "success": false, "errors": [{"code": 5006}]})
            .to_string()
            .into_bytes();

        assert!(matches!(
            parse(&failed, Question::Noul, &[]),
            Err(AnswerError::Unsuccessful)
        ));
        assert!(matches!(
            parse(b"<html>", Question::Noul, &[]),
            Err(AnswerError::Json(_))
        ));
        assert!(matches!(
            parse(&workers_ai(&json!("plain text")), Question::Noul, &[]),
            Err(AnswerError::Shape)
        ));
    }
}
