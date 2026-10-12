//! A ticket's name as `anneal brief` and `anneal round` take it.
//!
//! An issue number (`36`) or a dag ticket id (`003-auth`). It names files
//! (`.anneal/briefs/<ticket>.md`), so it is checked before anything is read or written.

use std::fmt;
use std::str::FromStr;

/// The longest ticket name, in bytes.
const MAX_LEN: usize = 64;

/// The suffix that makes a review package's stem (`<ticket>.review.md`).
const REVIEW_SUFFIX: &str = ".review";

/// Names Windows reserves for devices, whatever follows a `.`: `con.md` is no file there.
const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A ticket name: 1 to 64 ASCII letters, digits, `.`, `_` or `-`, not starting with `.`, so it
/// is a plain file name stem on every platform and can never name `.` or `..`.
///
/// Letters are lowered, so names that differ only in case, which share a file on APFS and NTFS, are one
/// ticket everywhere. It never ends in `.review` (its brief would be another ticket's review
/// package) and is no Windows device name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket(String);

impl Ticket {
    /// The name, lower case.
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

/// What is wrong with a name that is not a ticket name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Empty, too long, a character outside the set, or a leading `.`.
    Shape,
    /// It ends in `.review`.
    ReviewSuffix,
    /// It is a Windows device name (before any `.`).
    DeviceName,
}

impl Problem {
    const fn describe(self) -> &'static str {
        match self {
            Self::Shape => {
                "use 1 to 64 ASCII letters, digits, `.`, `_` or `-`, not starting with `.` (an \
                 issue number or a dag ticket id)"
            }
            Self::ReviewSuffix => {
                "a name ending in `.review` would share its brief's file with another ticket's \
                 review package"
            }
            Self::DeviceName => {
                "CON, PRN, AUX, NUL, COM1 to COM9 and LPT1 to LPT9 are device names on Windows, \
                 so no file can be named after them"
            }
        }
    }
}

/// A name that is not a ticket name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ticket {id:?}: {}", problem.describe())]
pub struct TicketError {
    /// The name that was given.
    pub id: String,
    /// What is wrong with it.
    pub problem: Problem,
}

impl FromStr for Ticket {
    type Err = TicketError;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        let allowed = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-');
        let refused = |problem| {
            Err(TicketError {
                id: id.to_owned(),
                problem,
            })
        };
        if !(1..=MAX_LEN).contains(&id.len()) || id.starts_with('.') || !id.bytes().all(allowed) {
            return refused(Problem::Shape);
        }
        let name = id.to_ascii_lowercase();
        if name.ends_with(REVIEW_SUFFIX) {
            return refused(Problem::ReviewSuffix);
        }
        let device = name.split('.').next().unwrap_or(&name);
        if DEVICE_NAMES.contains(&device) {
            return refused(Problem::DeviceName);
        }
        Ok(Self(name))
    }
}

#[cfg(test)]
mod tests {
    use super::{Problem, Ticket, TicketError};

    fn refused(id: &str, problem: Problem) -> Result<Ticket, TicketError> {
        Err(TicketError {
            id: id.to_owned(),
            problem,
        })
    }

    #[test]
    fn issue_numbers_and_dag_ids_are_tickets() {
        for id in ["36", "003-auth", "t1", "v0.1_story.7", "console", "com10"] {
            let ticket: Ticket = id.parse().expect("a plain id");
            assert_eq!(ticket.as_str(), id, "kept as written");
        }
    }

    #[test]
    fn letter_case_is_folded_so_a_ticket_has_one_file_on_every_file_system() {
        let upper: Ticket = "T1-Auth".parse().expect("a plain id");
        let lower: Ticket = "t1-auth".parse().expect("a plain id");

        assert_eq!(
            (upper.as_str(), &upper),
            ("t1-auth", &lower),
            "APFS and NTFS would give both one file, so they are one ticket"
        );
    }

    #[test]
    fn a_name_that_could_leave_the_briefs_directory_is_refused() {
        for id in ["", ".", "..", ".hidden", "a/b", "a\\b", "a b", "../x", "é"] {
            assert_eq!(id.parse::<Ticket>(), refused(id, Problem::Shape), "{id:?}");
        }
    }

    #[test]
    fn a_name_longer_than_64_bytes_is_refused() {
        let long = "a".repeat(65);
        assert_eq!(
            long.parse::<Ticket>(),
            refused(&long, Problem::Shape),
            "too long for a file name stem"
        );
        assert!("a".repeat(64).parse::<Ticket>().is_ok(), "64 is the limit");
    }

    #[test]
    fn a_name_whose_brief_would_be_another_ticket_s_review_package_is_refused() {
        for id in ["36.review", "36.REVIEW", "x.Review"] {
            assert_eq!(
                id.parse::<Ticket>(),
                refused(id, Problem::ReviewSuffix),
                "{id:?}.md is the review package of the ticket before the suffix"
            );
        }
    }

    #[test]
    fn a_windows_device_name_is_refused_in_any_case_and_with_any_extension() {
        for id in [
            "con", "CON", "prn", "Aux", "nul", "com1", "COM9", "lpt1", "Lpt9", "nul.x", "con.36",
        ] {
            assert_eq!(
                id.parse::<Ticket>(),
                refused(id, Problem::DeviceName),
                "{id:?}.md cannot be a file on Windows"
            );
        }
    }
}
