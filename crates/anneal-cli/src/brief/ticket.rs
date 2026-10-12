//! A ticket's name as `anneal brief` and `anneal round` take it.
//!
//! An issue number (`36`) or a dag ticket id (`003-auth`). It names files
//! (`.anneal/briefs/<ticket>.md`), so it is checked before anything is read or written.

use std::fmt;
use std::str::FromStr;

/// The longest ticket name, in bytes.
const MAX_LEN: usize = 64;

/// A ticket name: 1 to 64 ASCII letters, digits, `.`, `_` or `-`, not starting with `.`, so it
/// is a plain file name stem on every platform and can never name `.` or `..`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket(String);

impl Ticket {
    /// The name as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A name that is not a ticket name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "ticket {id:?}: use 1 to {MAX_LEN} ASCII letters, digits, `.`, `_` or `-`, not starting \
     with `.` (an issue number or a dag ticket id)"
)]
pub struct TicketError {
    /// The name that was given.
    pub id: String,
}

impl FromStr for Ticket {
    type Err = TicketError;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        let allowed = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-');
        let valid =
            (1..=MAX_LEN).contains(&id.len()) && !id.starts_with('.') && id.bytes().all(allowed);
        if valid {
            Ok(Self(id.to_owned()))
        } else {
            Err(TicketError { id: id.to_owned() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Ticket, TicketError};

    #[test]
    fn issue_numbers_and_dag_ids_are_tickets() {
        for id in ["36", "003-auth", "T1", "v0.1_story.7"] {
            let ticket: Ticket = id.parse().expect("a plain id");
            assert_eq!(ticket.as_str(), id, "kept as written");
        }
    }

    #[test]
    fn a_name_that_could_leave_the_briefs_directory_is_refused() {
        for id in ["", ".", "..", ".hidden", "a/b", "a\\b", "a b", "../x", "é"] {
            assert_eq!(
                id.parse::<Ticket>(),
                Err(TicketError { id: id.to_owned() }),
                "{id:?}"
            );
        }
    }

    #[test]
    fn a_name_longer_than_64_bytes_is_refused() {
        let long = "a".repeat(65);
        assert_eq!(
            long.parse::<Ticket>(),
            Err(TicketError { id: long.clone() }),
            "too long for a file name stem"
        );
        assert!("a".repeat(64).parse::<Ticket>().is_ok(), "64 is the limit");
    }
}
