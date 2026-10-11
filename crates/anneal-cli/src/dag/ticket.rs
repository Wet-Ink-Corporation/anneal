//! Tickets as every tracker hands them over, and the order they are listed in.

use std::cmp::Ordering;
use std::fmt;

use serde::Serialize;

/// A ticket's identity within its tracker: a markdown file's stem (`003-auth`) or an issue
/// number (`19`).
///
/// Ids order the way people number tickets: an id of ASCII digits only that fits a `u64` is a
/// number, numbers come first by value, and every other id follows by bytes. Two ids with the
/// same value (`01`, `1`) are still different ids and order by bytes, so the order is total
/// and agrees with `Eq`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TicketId(String);

impl TicketId {
    /// The id `id`, as the tracker writes it.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id's value when it is digits only and fits a `u64`.
    fn number(&self) -> Option<u64> {
        let digits = !self.0.is_empty() && self.0.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| self.0.parse().ok()).flatten()
    }
}

impl Ord for TicketId {
    fn cmp(&self, other: &Self) -> Ordering {
        let by_bytes = || self.0.as_bytes().cmp(other.0.as_bytes());
        match (self.number(), other.number()) {
            (Some(left), Some(right)) => left.cmp(&right).then_with(by_bytes),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => by_bytes(),
        }
    }
}

impl PartialOrd for TicketId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a ticket still has work in it. A closed set: a tracker maps its own states onto
/// these two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not finished: it may block others and may itself be ready.
    Open,
    /// Finished: it blocks nothing and is never in the frontier.
    Done,
}

/// One ticket as a [`Tracker`](crate::dag::tracker::Tracker) reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// Unique within the tracker's tickets.
    pub id: TicketId,
    /// One-line title.
    pub title: String,
    /// The ticket text the estimate reads.
    pub body: String,
    /// The tickets that must be done first, as written (they may name tickets that do not
    /// exist; `dag check` reports those).
    pub blocked_by: Vec<TicketId>,
    /// Files the tracker lists explicitly (markdown `files:`); files named in the body are
    /// found by the estimate.
    pub files: Vec<String>,
    /// Open or done.
    pub status: Status,
}

#[cfg(test)]
mod tests {
    use super::TicketId;

    fn sorted(ids: &[&str]) -> Vec<String> {
        let mut ids: Vec<TicketId> = ids.iter().map(|id| TicketId::new(*id)).collect();
        ids.sort();
        ids.iter().map(|id| id.as_str().to_owned()).collect()
    }

    #[test]
    fn numeric_ids_sort_by_value_before_other_ids() {
        assert_eq!(
            sorted(&["b", "10", "a", "9", "002-auth", "1"]),
            ["1", "9", "10", "002-auth", "a", "b"],
            "digits-only ids by value, then the rest by bytes"
        );
    }

    #[test]
    fn a_numeric_tie_breaks_by_bytes() {
        assert_eq!(
            sorted(&["1", "01", "001"]),
            ["001", "01", "1"],
            "same value"
        );
        assert_ne!(
            TicketId::new("01"),
            TicketId::new("1"),
            "still distinct ids"
        );
    }

    #[test]
    fn an_id_too_large_for_u64_sorts_as_text() {
        assert_eq!(
            sorted(&["99999999999999999999", "5"]),
            ["5", "99999999999999999999"],
            "past u64 it is text, after every number"
        );
    }
}
