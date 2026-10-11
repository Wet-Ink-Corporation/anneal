//! The envelope every stored event travels in: the event (its `type` and payload, flattened)
//! plus who wrote it, from where, and when.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::event::{EventKind, LedgerEvent};
use crate::tag::{LedgerTag, TagError, TagKey};

/// Why an event does not match the ledger schema.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The JSON is not an event: an unknown `type`, a missing or mistyped field, or a `ts`
    /// that is not RFC 3339.
    #[error("not a ledger event")]
    Json(#[source] serde_json::Error),
    /// The event is not a JSON object.
    #[error("an event must be a JSON object")]
    NotAnObject,
    /// The event carries a key its type does not define.
    #[error("unknown field {field:?} for event type {kind}")]
    UnknownField {
        /// The event's type.
        kind: EventKind,
        /// The key that is not part of it.
        field: String,
    },
    /// A tag value (session, worktree or subject) is not a valid tag.
    #[error(transparent)]
    Tag(#[from] TagError),
}

/// Why a timestamp could not be read or made.
#[derive(Debug, thiserror::Error)]
pub enum TimestampError {
    /// The text is not an RFC 3339 timestamp.
    #[error("ts {value:?} is not an RFC 3339 timestamp")]
    Parse {
        /// The text that was given.
        value: String,
        /// What the parser refused.
        #[source]
        source: time::error::Parse,
    },
    /// The clock's time cannot be written as RFC 3339 (a year past 9999).
    #[error("the system clock cannot be written as an RFC 3339 timestamp")]
    Clock(#[source] time::error::Format),
}

/// An RFC 3339 timestamp, kept as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Timestamp(String);

impl Timestamp {
    /// The current UTC time to the second, e.g. `2026-10-11T09:30:00Z`.
    ///
    /// # Errors
    /// [`TimestampError::Clock`] when the clock reads past the year 9999.
    pub fn now() -> Result<Self, TimestampError> {
        OffsetDateTime::now_utc()
            .truncate_to_second()
            .format(&Rfc3339)
            .map(Self)
            .map_err(TimestampError::Clock)
    }

    /// The timestamp as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Timestamp {
    type Error = TimestampError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match OffsetDateTime::parse(&value, &Rfc3339) {
            Ok(_) => Ok(Self(value)),
            Err(source) => Err(TimestampError::Parse { value, source }),
        }
    }
}

/// One stored event: the event, flattened, and its context.
///
/// On the wire: `{"type": "task_done", "task": "T1", "actor": "cli", "session_id": "s1",
/// "worktree": null, "branch": null, "head": null, "ts": "2026-10-11T09:30:00Z"}`.
/// Absent optional fields are written as `null` and read back from either `null` or a
/// missing key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// The event, flattened into the envelope's JSON object.
    #[serde(flatten)]
    pub event: LedgerEvent,
    /// Who wrote it: `cli`, `hook`, `agent` or `human:<name>`.
    pub actor: String,
    /// The writing agent session's id, if any. Tagged `session:`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The directory name of the writer's git worktree, if any. Tagged `worktree:`.
    #[serde(default)]
    pub worktree: Option<String>,
    /// The checked-out branch, if any.
    #[serde(default)]
    pub branch: Option<String>,
    /// The commit at `HEAD`, if any.
    #[serde(default)]
    pub head: Option<String>,
    /// When it was written, RFC 3339.
    pub ts: Timestamp,
}

impl EventEnvelope {
    /// An envelope for `event` with no session, worktree, branch or head.
    #[must_use]
    pub fn new(event: LedgerEvent, actor: impl Into<String>, ts: Timestamp) -> Self {
        Self {
            event,
            actor: actor.into(),
            session_id: None,
            worktree: None,
            branch: None,
            head: None,
            ts,
        }
    }

    /// Reads an event from JSON bytes; see [`EventEnvelope::from_value`].
    ///
    /// # Errors
    /// As [`EventEnvelope::from_value`]; also [`SchemaError::Json`] for bytes that are not JSON.
    pub fn parse(json: &[u8]) -> Result<Self, SchemaError> {
        let value: Value = serde_json::from_slice(json).map_err(SchemaError::Json)?;
        Self::from_value(&value)
    }

    /// Reads an event from a JSON value, strictly: the `type` must be registered, every field
    /// it requires present and well-typed, no key outside the envelope and the type's payload,
    /// `ts` RFC 3339, and every tag value valid.
    ///
    /// # Errors
    /// [`SchemaError::NotAnObject`], [`SchemaError::Json`] (unknown type, missing or mistyped
    /// field, bad `ts`), [`SchemaError::UnknownField`] or [`SchemaError::Tag`].
    pub fn from_value(value: &Value) -> Result<Self, SchemaError> {
        let Value::Object(input) = value else {
            return Err(SchemaError::NotAnObject);
        };
        let envelope = Self::deserialize(value).map_err(SchemaError::Json)?;
        // serde cannot deny unknown fields through `flatten`, so compare against the keys the
        // envelope writes back: every field, `null` included.
        let written = serde_json::to_value(&envelope).map_err(SchemaError::Json)?;
        if let Some(field) = input.keys().find(|key| written.get(key.as_str()).is_none()) {
            return Err(SchemaError::UnknownField {
                kind: envelope.event.kind(),
                field: field.clone(),
            });
        }
        envelope.tags()?;
        Ok(envelope)
    }

    /// The envelope as the JSON bytes the store keeps.
    ///
    /// # Errors
    /// Serialising strings, numbers and enums does not fail in practice; the error is returned
    /// rather than unwrapped.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// The store tags: `session:` and `worktree:` when set, and the subject tag of the event's
    /// type when the event names a subject.
    ///
    /// # Errors
    /// [`TagError::Value`] when a value is not a valid tag value.
    pub fn tags(&self) -> Result<Vec<LedgerTag>, TagError> {
        let context = [
            (TagKey::Session, self.session_id.as_deref()),
            (TagKey::Worktree, self.worktree.as_deref()),
        ];
        let subject = self.event.subject_value();
        context
            .into_iter()
            .chain([(self.event.kind().subject(), subject.as_deref())])
            .filter_map(|(key, value)| value.map(|value| LedgerTag::new(key, value)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{EventEnvelope, SchemaError, Timestamp, TimestampError};
    use crate::event::{EventKind, LedgerEvent, TaskDone, Verdict, VerdictSet};
    use crate::tag::{TagError, TagKey};
    use serde_json::json;

    fn ts() -> Timestamp {
        Timestamp::try_from("2026-10-11T09:30:00Z".to_owned()).expect("a valid timestamp")
    }

    fn task_done(task: &str) -> EventEnvelope {
        let mut envelope = EventEnvelope::new(
            LedgerEvent::TaskDone(TaskDone {
                task: task.to_owned(),
            }),
            "cli",
            ts(),
        );
        envelope.session_id = Some("s1".to_owned());
        envelope
    }

    #[test]
    fn an_envelope_round_trips_with_the_documented_keys() {
        let envelope = task_done("T1");

        let json = envelope.to_json().expect("serialises");
        let parsed = EventEnvelope::parse(&json).expect("parses back");

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&json).expect("JSON"),
            json!({
                "type": "task_done", "task": "T1", "actor": "cli", "session_id": "s1",
                "worktree": null, "branch": null, "head": null, "ts": "2026-10-11T09:30:00Z",
            }),
            "type and payload are flattened beside the context"
        );
        assert_eq!(parsed, envelope, "round trip");
    }

    #[test]
    fn missing_optional_fields_read_as_none() {
        let parsed = EventEnvelope::from_value(&json!({
            "type": "verdict_set", "pr": 12, "verdict": "noise_only", "actor": "hook",
            "ts": "2026-10-11T09:30:00+02:00",
        }))
        .expect("optional fields may be left out");

        assert_eq!(
            parsed.event,
            LedgerEvent::VerdictSet(VerdictSet {
                pr: 12,
                verdict: Verdict::NoiseOnly
            }),
            "payload read"
        );
        assert_eq!(parsed.session_id, None, "no session");
    }

    #[test]
    fn unknown_types_and_missing_fields_are_json_schema_errors() {
        let unknown = EventEnvelope::from_value(&json!({
            "type": "task_started", "actor": "cli", "ts": "2026-10-11T09:30:00Z",
        }));
        let missing = EventEnvelope::from_value(&json!({
            "type": "task_done", "actor": "cli", "ts": "2026-10-11T09:30:00Z",
        }));

        assert!(
            matches!(&unknown, Err(SchemaError::Json(e)) if e.to_string().contains("task_started")),
            "{unknown:?}"
        );
        assert!(
            matches!(&missing, Err(SchemaError::Json(e)) if e.to_string().contains("task")),
            "{missing:?}"
        );
    }

    #[test]
    fn a_key_outside_the_type_is_an_unknown_field() {
        let result = EventEnvelope::from_value(&json!({
            "type": "task_done", "task": "T1", "plan": "p.md", "actor": "cli",
            "ts": "2026-10-11T09:30:00Z",
        }));

        assert!(
            matches!(&result, Err(SchemaError::UnknownField { kind: EventKind::TaskDone, field }) if field == "plan"),
            "{result:?}"
        );
    }

    #[test]
    fn a_bad_timestamp_or_a_non_object_is_refused() {
        let bad_ts = EventEnvelope::from_value(&json!({
            "type": "task_done", "task": "T1", "actor": "cli", "ts": "yesterday",
        }));
        let array = EventEnvelope::from_value(&json!([1, 2]));

        assert!(
            matches!(&bad_ts, Err(SchemaError::Json(e)) if e.to_string().contains("RFC 3339")),
            "{bad_ts:?}"
        );
        assert!(
            matches!(array, Err(SchemaError::NotAnObject)),
            "not an object"
        );
        assert!(
            matches!(
                Timestamp::try_from("2026-13-01T00:00:00Z".to_owned()),
                Err(TimestampError::Parse { .. })
            ),
            "month 13"
        );
    }

    #[test]
    fn tags_are_session_worktree_and_the_subject() {
        let mut envelope = task_done("T1");
        envelope.worktree = Some("wt11".to_owned());

        let tags: Vec<String> = envelope
            .tags()
            .expect("valid tags")
            .iter()
            .map(ToString::to_string)
            .collect();

        assert_eq!(tags, ["session:s1", "worktree:wt11", "task:T1"], "tags");
    }

    #[test]
    fn an_empty_subject_is_a_tag_error() {
        let result = EventEnvelope::from_value(&json!({
            "type": "task_done", "task": "", "actor": "cli", "ts": "2026-10-11T09:30:00Z",
        }));

        assert!(
            matches!(
                result,
                Err(SchemaError::Tag(TagError::Value {
                    key: TagKey::Task,
                    ..
                }))
            ),
            "an empty task id cannot be a tag"
        );
    }

    #[test]
    fn now_is_a_parseable_second_precision_timestamp() {
        let now = Timestamp::now().expect("the clock is before 9999");

        assert!(now.as_str().ends_with('Z'), "UTC: {}", now.as_str());
        assert!(
            !now.as_str().contains('.'),
            "whole seconds: {}",
            now.as_str()
        );
        assert!(
            Timestamp::try_from(now.as_str().to_owned()).is_ok(),
            "now parses back"
        );
    }
}
