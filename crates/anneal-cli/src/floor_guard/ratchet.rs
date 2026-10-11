//! The ratchet file, `.anneal/ratchets.json`: numeric floors that may only move the good way.
//!
//! ```json
//! {"version": 1, "ratchets": {"warnings": {"value": 12, "direction": "down"}}}
//! ```
//!
//! `down` means lower is better (a warnings count), `up` higher is better (coverage). The file
//! is read strictly ([`Ratchets::parse`]); [`loosened`] compares the merge base's copy with the
//! working tree's. floor-guard never computes the metrics: it guards the committed numbers.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::detect::{Category, FloorFinding};

/// Where the ratchet file lives, relative to the repository root.
pub(crate) const RATCHETS_PATH: &str = ".anneal/ratchets.json";

/// The only format version.
const VERSION: u64 = 1;

/// Most ratchets one file may hold.
pub(crate) const MAX_RATCHETS: usize = 256;

/// Longest ratchet name, in bytes.
const MAX_NAME_BYTES: usize = 64;

/// Which way a ratchet improves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Direction {
    /// Lower is better, such as a warnings count.
    Down,
    /// Higher is better, such as coverage.
    Up,
}

impl Direction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Up => "up",
        }
    }

    /// True when moving from `from` to `to` is the bad way.
    fn worse(self, from: f64, to: f64) -> bool {
        match self {
            Self::Down => to > from,
            Self::Up => to < from,
        }
    }
}

/// One floor.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ratchet {
    /// The committed number.
    pub(crate) value: f64,
    /// Which way it improves.
    pub(crate) direction: Direction,
}

/// The file as written; [`Ratchets::parse`] checks what serde cannot.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RatchetsDto {
    version: u64,
    ratchets: BTreeMap<String, Ratchet>,
}

/// A valid ratchet file: its ratchets by name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ratchets(BTreeMap<String, Ratchet>);

/// Why a ratchet file is not valid.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RatchetError {
    /// Not JSON of the expected shape: a missing or unknown field, a value that is not a
    /// number, a direction other than `down` or `up`.
    #[error("not a ratchet file")]
    Json(#[source] serde_json::Error),
    /// A version other than 1.
    #[error("version {found} is not {VERSION}")]
    Version {
        /// The version written.
        found: u64,
    },
    /// A name that is empty, too long, or holds a character other than ASCII letters, digits,
    /// `_`, `.` and `-`.
    #[error("ratchet name {name:?} is not 1 to {MAX_NAME_BYTES} of A-Z a-z 0-9 _ . -")]
    Name {
        /// The name written.
        name: String,
    },
    /// More than [`MAX_RATCHETS`] ratchets.
    #[error("{count} ratchets, more than {MAX_RATCHETS}")]
    TooMany {
        /// How many there are.
        count: usize,
    },
}

impl Ratchets {
    /// Reads `text` as a version 1 ratchet file.
    ///
    /// # Errors
    /// The [`RatchetError`] naming the first thing that is not valid.
    pub(crate) fn parse(text: &str) -> Result<Self, RatchetError> {
        let dto: RatchetsDto = serde_json::from_str(text).map_err(RatchetError::Json)?;
        if dto.version != VERSION {
            return Err(RatchetError::Version { found: dto.version });
        }
        if dto.ratchets.len() > MAX_RATCHETS {
            return Err(RatchetError::TooMany {
                count: dto.ratchets.len(),
            });
        }
        if let Some(name) = dto.ratchets.keys().find(|name| !valid_name(name)) {
            return Err(RatchetError::Name {
                // The error owns the name it reports.
                name: name.clone(),
            });
        }
        Ok(Self(dto.ratchets))
    }

    /// The ratchet called `name`.
    pub(crate) fn get(&self, name: &str) -> Option<&Ratchet> {
        self.0.get(name)
    }

    /// How many ratchets there are.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// How a ratchet was loosened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Change {
    /// Its value moved the bad way for `direction`.
    Moved {
        /// The base value.
        from: f64,
        /// The working tree's value.
        to: f64,
        /// Its direction (the same in both).
        direction: Direction,
    },
    /// It is gone from the working tree's file.
    Removed,
    /// Its direction changed, which turns the floor into a ceiling.
    Redirected {
        /// The base direction.
        from: Direction,
        /// The working tree's direction.
        to: Direction,
    },
}

/// A ratchet of the base that the working tree loosened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Loosened<'a> {
    /// The ratchet's name.
    pub(crate) name: &'a str,
    /// How.
    pub(crate) change: Change,
    /// The base ratchet.
    base: Ratchet,
}

impl Loosened<'_> {
    /// One line for a human.
    pub(crate) fn message(&self) -> String {
        let name = self.name;
        match self.change {
            Change::Moved {
                from,
                to,
                direction: Direction::Down,
            } => format!("raises ratchet {name} from {from} to {to} (down: lower is better)"),
            Change::Moved {
                from,
                to,
                direction: Direction::Up,
            } => format!("lowers ratchet {name} from {from} to {to} (up: higher is better)"),
            Change::Removed => format!(
                "removes ratchet {name} ({}, {})",
                self.base.value,
                self.base.direction.as_str()
            ),
            Change::Redirected { from, to } => format!(
                "turns ratchet {name} from {} to {}",
                from.as_str(),
                to.as_str()
            ),
        }
    }
}

/// Every ratchet of `base` that `now` loosened, by name. A ratchet new in `now` is not judged:
/// adding a floor never lowers one.
pub(crate) fn loosened<'a>(base: &'a Ratchets, now: &Ratchets) -> Vec<Loosened<'a>> {
    base.0
        .iter()
        .filter_map(|(name, &old)| {
            let change = match now.get(name) {
                None => Change::Removed,
                Some(new) if new.direction != old.direction => Change::Redirected {
                    from: old.direction,
                    to: new.direction,
                },
                Some(new) if old.direction.worse(old.value, new.value) => Change::Moved {
                    from: old.value,
                    to: new.value,
                    direction: old.direction,
                },
                Some(_) => return None,
            };
            Some(Loosened {
                name,
                change,
                base: old,
            })
        })
        .collect()
}

/// The working tree's ratchet file, as read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Now<'a> {
    /// It is gone (or is not a regular file).
    Absent,
    /// It is over the read cap.
    TooLarge,
    /// Its content.
    Text(&'a str),
}

/// The floor findings for the working tree's ratchet file `now` against the merge base's
/// `base`: one per loosened ratchet, at the line that names it; one per ratchet when the file
/// is gone; and one for the whole file when it cannot be read as a ratchet file, so breaking
/// the file never unblocks a floor.
pub(crate) fn findings(base: &Ratchets, now: Now<'_>) -> Vec<FloorFinding> {
    let finding = |line, message| FloorFinding {
        category: Category::RatchetLoosened,
        file: RATCHETS_PATH.to_owned(),
        line,
        message,
    };
    let text = match now {
        Now::Text(text) => text,
        Now::TooLarge => {
            let message = "the ratchet file is over 1 MiB, so its floors cannot be read".to_owned();
            return vec![finding(None, message)];
        }
        Now::Absent => {
            let removed = Ratchets(BTreeMap::new());
            return loosened(base, &removed)
                .iter()
                .map(|loose| finding(None, loose.message()))
                .collect();
        }
    };
    match Ratchets::parse(text) {
        Ok(now) => loosened(base, &now)
            .iter()
            .map(|loose| finding(line_of(text, loose.name), loose.message()))
            .collect(),
        Err(error) => vec![finding(
            None,
            format!(
                "the ratchet file is no longer valid ({}), so its floors cannot be read",
                crate::error_chain::reason(&error)
            ),
        )],
    }
}

/// The 1-based line of `text` where `"name"` is a key (followed by `:`), if any.
pub(crate) fn line_of(text: &str, name: &str) -> Option<u64> {
    let quoted = format!("\"{name}\"");
    let index = text.lines().position(|line| {
        line.match_indices(&quoted).any(|(at, _)| {
            line.get(at.saturating_add(quoted.len())..)
                .is_some_and(|rest| rest.trim_start().starts_with(':'))
        })
    })?;
    u64::try_from(index).ok()?.checked_add(1)
}

#[cfg(test)]
mod tests;
