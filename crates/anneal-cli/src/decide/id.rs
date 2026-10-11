//! Decision ids, unique per row, and the outcome labels `decide label` attaches to them.
//!
//! An id is `d<unix-millis>-<pid>-<n>`: the time of the decision, the process, and a counter
//! within the process, so two decisions never share one, even in the same millisecond.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Longest id or outcome label accepted.
const MAX_BYTES: usize = 64;

/// Distinguishes the decisions of one process. Process-wide on purpose: an id from any caller
/// in this process must differ from every other (the same shape as `baseline`'s temporary
/// file names).
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Why `decide label` refused its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidLabel {
    /// Not an id `anneal decide` writes.
    #[error("not a decision id: expected d<millis>-<pid>-<n>")]
    Id,
    /// Not 1 to 64 of `[A-Za-z0-9_.:-]`.
    #[error("not an outcome label: use 1 to 64 of A-Z, a-z, 0-9, `_`, `.`, `:` and `-`")]
    Outcome,
}

/// A decision's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DecisionId(String);

impl DecisionId {
    /// A new id for a decision made at `unix_millis`.
    #[must_use]
    pub fn generate(unix_millis: u64) -> Self {
        // Relaxed: the counter only hands out distinct numbers; it orders no other memory.
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(format!("d{unix_millis}-{}-{n}", std::process::id()))
    }

    /// The id as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DecisionId {
    type Err = InvalidLabel;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        let valid = text.len() <= MAX_BYTES
            && text.strip_prefix('d').is_some_and(|rest| {
                matches!(rest.split('-').collect::<Vec<_>>().as_slice(),
                    [millis, pid, n] if digits(millis) && digits(pid) && digits(n))
            });
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(InvalidLabel::Id)
        }
    }
}

impl fmt::Display for DecisionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What turned out to be right for a decision, as `anneal calibrate` will read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OutcomeLabel(String);

impl OutcomeLabel {
    /// The label as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for OutcomeLabel {
    type Err = InvalidLabel;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let valid = (1..=MAX_BYTES).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b));
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(InvalidLabel::Outcome)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{DecisionId, InvalidLabel, OutcomeLabel};

    #[test]
    fn generated_ids_are_distinct_and_parse_back() {
        let ids: Vec<DecisionId> = (0..100)
            .map(|_| DecisionId::generate(1_760_000_000_000))
            .collect();

        let distinct: BTreeSet<&str> = ids.iter().map(DecisionId::as_str).collect();
        assert_eq!(
            distinct.len(),
            ids.len(),
            "same millisecond, still distinct"
        );
        for id in &ids {
            assert_eq!(id.as_str().parse::<DecisionId>().as_ref(), Ok(id), "{id}");
        }
    }

    #[test]
    fn an_id_starts_with_the_time_so_ids_sort_by_time_within_a_width() {
        let id = DecisionId::generate(1_760_000_000_123);

        assert!(id.as_str().starts_with("d1760000000123-"), "{id}");
    }

    #[test]
    fn malformed_ids_are_refused() {
        for bad in [
            "", "d", "d1-2", "x1-2-3", "d1-2-3-4", "d1-2-x", "d-1-2", "d1--2",
        ] {
            assert_eq!(bad.parse::<DecisionId>(), Err(InvalidLabel::Id), "{bad:?}");
        }
    }

    #[test]
    fn outcome_labels_are_short_words() {
        for good in [
            "destructive",
            "not_destructive",
            "deny",
            "true",
            "0",
            "re-open",
            "a.b:c",
        ] {
            assert!(good.parse::<OutcomeLabel>().is_ok(), "{good:?} is accepted");
        }
        for bad in ["", "two words", "new\nline", &"x".repeat(65)] {
            assert_eq!(
                bad.parse::<OutcomeLabel>(),
                Err(InvalidLabel::Outcome),
                "{bad:?}"
            );
        }
    }
}
