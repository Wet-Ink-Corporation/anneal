//! The compact state sent to Clef: a JSON object with stable fields first and volatile ones
//! last, long strings cut, and a size cap.
//!
//! The order keeps the request's prefix the same from call to call: [`STABLE_FIRST`] in that
//! order, then the other fields by name, then the volatile ones ([`VOLATILE`], or a name ending
//! in one of [`VOLATILE_SUFFIXES`]) by name. Nested objects are ordered by name. Every string
//! is trimmed and cut to [`MAX_STRING_CHARS`] characters, marked with `…`.

use serde_json::{Map, Value};

use crate::content_hash;

/// Most bytes of `--state` read (H-13).
pub const MAX_RAW_BYTES: usize = 64 * 1024;

/// Most bytes of the compacted state sent to Clef and hashed.
pub const MAX_COMPACT_BYTES: usize = 16 * 1024;

/// Most characters kept of one string.
pub const MAX_STRING_CHARS: usize = 1024;

/// Fields that lead, in this order.
pub const STABLE_FIRST: [&str; 3] = ["tool", "phase", "input"];

/// Fields that change from call to call and so come last.
pub const VOLATILE: [&str; 13] = [
    "sha",
    "commit",
    "head",
    "timestamp",
    "ts",
    "time",
    "date",
    "counter",
    "count",
    "turn",
    "seq",
    "session_id",
    "nonce",
];

/// Name endings that mark a field as volatile.
pub const VOLATILE_SUFFIXES: [&str; 6] = ["_sha", "_at", "_ts", "_time", "_count", "_counter"];

/// Why `--state` was refused.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// Not JSON.
    #[error("--state is not JSON")]
    Json(#[source] serde_json::Error),
    /// JSON, but not an object.
    #[error("--state must be a JSON object")]
    NotAnObject,
    /// Over [`MAX_RAW_BYTES`] as given, or over [`MAX_COMPACT_BYTES`] once compacted.
    #[error("--state is over {limit} bytes")]
    TooLarge {
        /// The limit passed.
        limit: usize,
    },
}

/// The state as sent to Clef and hashed into the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactState {
    json: String,
}

impl CompactState {
    /// Parses `raw` and compacts it.
    ///
    /// # Errors
    /// [`StateError`] when `raw` is over its cap, not a JSON object, or over the compacted cap.
    pub fn parse(raw: &str) -> Result<Self, StateError> {
        if raw.len() > MAX_RAW_BYTES {
            return Err(StateError::TooLarge {
                limit: MAX_RAW_BYTES,
            });
        }
        let Value::Object(fields) = serde_json::from_str(raw).map_err(StateError::Json)? else {
            return Err(StateError::NotAnObject);
        };
        let json = ordered_json(fields)?;
        if json.len() > MAX_COMPACT_BYTES {
            return Err(StateError::TooLarge {
                limit: MAX_COMPACT_BYTES,
            });
        }
        Ok(Self { json })
    }

    /// The compact JSON text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.json
    }

    /// `sha256:` and the hex SHA-256 of the compact text.
    #[must_use]
    pub fn sha256(&self) -> String {
        content_hash::sha256(self.json.as_bytes())
    }
}

/// Where a top-level field goes: its rank among [`STABLE_FIRST`], then the rest, then volatile.
fn rank(name: &str) -> (u8, usize) {
    if let Some(position) = STABLE_FIRST.iter().position(|stable| *stable == name) {
        (0, position)
    } else if VOLATILE.contains(&name) || VOLATILE_SUFFIXES.iter().any(|end| name.ends_with(end)) {
        (2, 0)
    } else {
        (1, 0)
    }
}

/// The fields as one JSON object in [`rank`] order, then by name, each value cut.
fn ordered_json(fields: Map<String, Value>) -> Result<String, StateError> {
    // `Map` iterates by name, and the sort is stable, so equal ranks stay in name order.
    let mut fields: Vec<(String, Value)> = fields.into_iter().collect();
    fields.sort_by_key(|(name, _)| rank(name));
    let mut json = String::from("{");
    for (index, (name, value)) in fields.into_iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&serde_json::to_string(&name).map_err(StateError::Json)?);
        json.push(':');
        json.push_str(&serde_json::to_string(&cut(value)).map_err(StateError::Json)?);
    }
    json.push('}');
    Ok(json)
}

/// `value` with every string trimmed and cut to [`MAX_STRING_CHARS`].
fn cut(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(cut_text(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(cut).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name, cut(value)))
                .collect(),
        ),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_)) => other,
    }
}

/// `text` trimmed, and cut to [`MAX_STRING_CHARS`] characters plus `…` when longer.
fn cut_text(text: &str) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(MAX_STRING_CHARS) {
        Some((end, _)) => {
            let kept = trimmed.get(..end).unwrap_or(trimmed);
            format!("{kept}…")
        }
        None => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{CompactState, MAX_STRING_CHARS, StateError};

    fn compact(raw: &str) -> Result<CompactState, StateError> {
        CompactState::parse(raw)
    }

    #[test]
    fn stable_fields_come_first_and_volatile_fields_last() -> Result<(), StateError> {
        let state = compact(
            r#"{"sha":"abc123","turn":7,"input":{"command":"ls"},"phase":"build","tool":"Bash","zeta":1,"alpha":2,"started_at":"t"}"#,
        )?;

        assert_eq!(
            state.as_str(),
            r#"{"tool":"Bash","phase":"build","input":{"command":"ls"},"alpha":2,"zeta":1,"sha":"abc123","started_at":"t","turn":7}"#
        );
        Ok(())
    }

    #[test]
    fn the_same_fields_in_any_order_compact_to_the_same_bytes_and_hash() -> Result<(), StateError> {
        let one = compact(r#"{"tool":"Bash","sha":"1","input":"x"}"#)?;
        let other = compact(r#"{ "input" : "x", "sha":"1", "tool":"Bash" }"#)?;

        assert_eq!(one.as_str(), other.as_str());
        assert_eq!(one.sha256(), other.sha256());
        assert!(one.sha256().starts_with("sha256:"), "{}", one.sha256());
        Ok(())
    }

    #[test]
    fn strings_are_trimmed_and_long_ones_cut_at_a_char_boundary() -> Result<(), StateError> {
        let long = "é".repeat(MAX_STRING_CHARS + 10);
        let raw =
            serde_json::json!({"input": {"command": format!("  {long}  ")}, "tool": " Bash "})
                .to_string();

        let state = compact(&raw)?;
        let value: serde_json::Value =
            serde_json::from_str(state.as_str()).map_err(StateError::Json)?;

        assert_eq!(value["tool"], "Bash");
        let command = value["input"]["command"].as_str().expect("a string");
        assert_eq!(
            command.chars().count(),
            MAX_STRING_CHARS + 1,
            "cut, plus the ellipsis"
        );
        assert!(command.ends_with('…'), "marked as cut");
        Ok(())
    }

    #[test]
    fn a_state_that_is_not_a_json_object_is_refused() {
        for raw in ["[1,2]", "\"text\"", "not json", ""] {
            assert!(
                matches!(
                    compact(raw),
                    Err(StateError::NotAnObject | StateError::Json(_))
                ),
                "{raw:?} is refused"
            );
        }
    }

    #[test]
    fn a_state_over_the_raw_cap_is_refused() {
        let raw = format!(r#"{{"input":"{}"}}"#, "x".repeat(super::MAX_RAW_BYTES));

        assert!(matches!(compact(&raw), Err(StateError::TooLarge { .. })));
    }

    #[test]
    fn a_compacted_state_over_its_cap_is_refused() {
        let fields: Vec<String> = (0..40)
            .map(|n| format!(r#""k{n}":"{}""#, "y".repeat(MAX_STRING_CHARS)))
            .collect();
        let raw = format!("{{{}}}", fields.join(","));

        assert!(matches!(compact(&raw), Err(StateError::TooLarge { .. })));
    }
}
