//! The tag vocabulary: `key:value` tags the store indexes events by.
//!
//! Every event carries `session:` and `worktree:` when its envelope names them, plus one
//! subject tag chosen by its type (`task:`, `plan:`, `pr:` or `point:`). The keys are a closed
//! set; values follow happenstance's tag rules (non-empty, no control characters, at most
//! [`MAX_TAG_LEN`](happenstance_core::MAX_TAG_LEN) bytes with the key).

use std::fmt;
use std::str::FromStr;

use happenstance_core::{InvalidTag, Tag};

/// A tag key in the ledger's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagKey {
    /// `session:<session_id>`
    Session,
    /// `worktree:<worktree>`
    Worktree,
    /// `task:<task id>`
    Task,
    /// `plan:<plan id>`
    Plan,
    /// `pr:<number>`
    Pr,
    /// `point:<gate or decision point>`
    Point,
}

impl TagKey {
    /// Every key, in documentation order.
    pub const ALL: [Self; 6] = [
        Self::Session,
        Self::Worktree,
        Self::Task,
        Self::Plan,
        Self::Pr,
        Self::Point,
    ];

    /// The key as stored, before the `:`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Worktree => "worktree",
            Self::Task => "task",
            Self::Plan => "plan",
            Self::Pr => "pr",
            Self::Point => "point",
        }
    }
}

impl fmt::Display for TagKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a tag could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    /// The text has no `:` separating key and value.
    #[error("a tag is written key:value, got {text:?}")]
    NotKeyValue {
        /// The text that was given.
        text: String,
    },
    /// The key is not in the vocabulary.
    #[error("unknown tag key {key:?}: expected session, worktree, task, plan, pr or point")]
    UnknownKey {
        /// The key that was given.
        key: String,
    },
    /// The value is not a valid tag value (empty, a control character, too long).
    #[error("invalid {key} tag value {value:?}")]
    Value {
        /// The tag's key.
        key: TagKey,
        /// The value that was refused.
        value: String,
        /// Why the store refuses it.
        #[source]
        source: InvalidTag,
    },
}

/// A validated `key:value` tag from the ledger's vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LedgerTag {
    key: TagKey,
    tag: Tag,
}

impl LedgerTag {
    /// The tag `key:value`.
    ///
    /// # Errors
    /// [`TagError::Value`] when `value` is not a valid tag value.
    pub fn new(key: TagKey, value: &str) -> Result<Self, TagError> {
        Tag::key_value(key.as_str(), value)
            .map(|tag| Self { key, tag })
            .map_err(|source| TagError::Value {
                key,
                value: value.to_owned(),
                source,
            })
    }

    /// The tag's key.
    #[must_use]
    pub const fn key(&self) -> TagKey {
        self.key
    }

    /// The whole tag, `key:value`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.tag.as_str()
    }

    /// The store's tag.
    pub(crate) const fn as_tag(&self) -> &Tag {
        &self.tag
    }

    /// The store's tag, by value.
    pub(crate) fn into_tag(self) -> Tag {
        self.tag
    }
}

impl fmt::Display for LedgerTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LedgerTag {
    type Err = TagError;

    /// Parses `key:value`, splitting at the first `:`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (key, value) = text.split_once(':').ok_or_else(|| TagError::NotKeyValue {
            text: text.to_owned(),
        })?;
        let key = TagKey::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == key)
            .ok_or_else(|| TagError::UnknownKey {
                key: key.to_owned(),
            })?;
        Self::new(key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::{LedgerTag, TagError, TagKey};

    #[test]
    fn key_value_text_parses_into_a_vocabulary_tag() {
        let tag: LedgerTag = "task:T1:a".parse().expect("a task tag");

        assert_eq!(tag.key(), TagKey::Task, "split at the first colon");
        assert_eq!(tag.as_str(), "task:T1:a", "the value keeps later colons");
    }

    #[test]
    fn unknown_keys_missing_colons_and_empty_values_are_refused() {
        assert!(
            matches!("item:x".parse::<LedgerTag>(), Err(TagError::UnknownKey { key }) if key == "item"),
            "item is not in the vocabulary"
        );
        assert!(
            matches!(
                "task".parse::<LedgerTag>(),
                Err(TagError::NotKeyValue { .. })
            ),
            "no colon"
        );
        assert!(
            matches!(
                "plan:".parse::<LedgerTag>(),
                Err(TagError::Value {
                    key: TagKey::Plan,
                    ..
                })
            ),
            "empty value"
        );
    }
}
