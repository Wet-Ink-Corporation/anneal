//! ADR numbering: `docs/adr/NNNN-<slug>.md` gets the next free number.
//!
//! An ADR file name is four or more ASCII digits, `-`, a slug and `.md`; other files in the
//! directory (a `README.md`, a template) are not ADRs. ADRs are ordered by number, then slug, so
//! two branches that both took the same number still list in one order after a merge. A new
//! title gets the highest number plus one (numbers are never reused), and a title already
//! recorded gets its existing ADR back, so asking twice gives the same answer. See
//! `docs/define.md`.

use std::fs;
use std::io;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};

/// The width numbers are zero-padded to.
const WIDTH: usize = 4;

/// One ADR, as named on disk.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Adr {
    // Field order is the sort order: number, then slug, then the name (which differs only in
    // padding when number and slug are equal).
    number: u32,
    slug: String,
    /// The file name as on disk, so `00012-x.md` is never renamed to `0012-x.md`.
    file_name: String,
}

/// Where the ADR for a title goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// The title is already recorded under this ADR.
    Existing(Adr),
    /// The title is new; this is the next free ADR.
    New(Adr),
}

/// Why an ADR could not be placed.
#[derive(Debug, thiserror::Error)]
pub enum AdrError {
    /// The title has no ASCII letter or digit to make a slug from.
    #[error("the title {title:?} has no ASCII letter or digit for a file name")]
    EmptySlug {
        /// The title as given.
        title: String,
    },
    /// A file named like an ADR has a number too large to count.
    #[error("{name}: the ADR number does not fit in 32 bits")]
    Number {
        /// The file name.
        name: String,
        /// The parse failure.
        #[source]
        source: ParseIntError,
    },
    /// The highest ADR number is already taken.
    #[error("no ADR number is left after {max}", max = u32::MAX)]
    Exhausted,
    /// The ADR directory could not be listed.
    #[error("listing {dir}", dir = dir.display())]
    Io {
        /// The directory.
        dir: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
}

impl Adr {
    /// The ADR named `file_name`, `None` when the name is not an ADR's.
    ///
    /// # Errors
    /// [`AdrError::Number`] when the name is an ADR's but its number does not fit in a `u32`.
    pub fn parse(file_name: &str) -> Result<Option<Self>, AdrError> {
        let Some((digits, slug)) = file_name
            .strip_suffix(".md")
            .and_then(|stem| stem.split_once('-'))
        else {
            return Ok(None);
        };
        let numbered = digits.len() >= WIDTH && digits.bytes().all(|b| b.is_ascii_digit());
        if !numbered || slug.is_empty() {
            return Ok(None);
        }
        let number = digits.parse().map_err(|source| AdrError::Number {
            name: file_name.to_owned(),
            source,
        })?;
        Ok(Some(Self {
            number,
            slug: slug.to_owned(),
            file_name: file_name.to_owned(),
        }))
    }

    /// A new ADR, named `NNNN-<slug>.md` with the number zero-padded to four digits.
    fn numbered(number: u32, slug: String) -> Self {
        let file_name = format!("{number:0WIDTH$}-{slug}.md");
        Self {
            number,
            slug,
            file_name,
        }
    }

    /// The ADR number.
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.number
    }

    /// The slug, as written in the file name.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// The file name: as found on disk for an existing ADR, `NNNN-<slug>.md` for a new one.
    #[must_use]
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
}

impl Placement {
    /// The ADR, existing or new.
    #[must_use]
    pub const fn adr(&self) -> &Adr {
        match self {
            Self::Existing(adr) | Self::New(adr) => adr,
        }
    }
}

/// The slug of `title`: lowercase ASCII letters and digits joined by `-`.
///
/// Every other run of characters becomes one `-`, with none at either end. Non-ASCII letters
/// are separators, so a file name reads the same on every file system.
///
/// # Errors
/// [`AdrError::EmptySlug`] when `title` has no ASCII letter or digit.
pub fn slug(title: &str) -> Result<String, AdrError> {
    let slug = slug_of(title);
    if slug.is_empty() {
        return Err(AdrError::EmptySlug {
            title: title.to_owned(),
        });
    }
    Ok(slug)
}

fn slug_of(text: &str) -> String {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

/// The ADRs among `names` (file names), sorted by number then slug, without duplicates.
///
/// # Errors
/// [`AdrError::Number`] when an ADR's number does not fit in a `u32`.
pub fn ordered<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Vec<Adr>, AdrError> {
    let mut adrs = Vec::new();
    for name in names {
        if let Some(adr) = Adr::parse(name)? {
            adrs.push(adr);
        }
    }
    adrs.sort();
    adrs.dedup();
    Ok(adrs)
}

/// Where the ADR for `title` goes among `existing`.
///
/// That is the first ADR whose slug matches the title's (compared as slugs, so `Use-Postgres`
/// matches "use postgres"), else a new ADR numbered one past the highest (`0001` when there
/// is none). The returned ADR is owned, so an existing one is copied out of `existing`.
///
/// # Errors
/// [`AdrError::EmptySlug`] when the title has no slug, [`AdrError::Exhausted`] when the
/// highest number is `u32::MAX`.
pub fn next(existing: &[Adr], title: &str) -> Result<Placement, AdrError> {
    let slug = slug(title)?;
    if let Some(found) = existing.iter().find(|adr| slug_of(&adr.slug) == slug) {
        return Ok(Placement::Existing(found.clone()));
    }
    let number = match existing.iter().map(|adr| adr.number).max() {
        None => 1,
        Some(max) => max.checked_add(1).ok_or(AdrError::Exhausted)?,
    };
    Ok(Placement::New(Adr::numbered(number, slug)))
}

/// Where the ADR for `title` goes in `dir` (normally `docs/adr`). Reads only file names and
/// writes nothing; a missing directory has no ADRs yet.
///
/// # Errors
/// [`AdrError::Io`] when `dir` exists but cannot be listed, and the errors of [`ordered`] and
/// [`next`].
pub fn place(dir: &Path, title: &str) -> Result<Placement, AdrError> {
    let io_error = |source| AdrError::Io {
        dir: dir.to_path_buf(),
        source,
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return next(&[], title),
        Err(error) => return Err(io_error(error)),
    };
    let mut names = Vec::new();
    for entry in entries {
        // A non-UTF-8 name keeps its number when read lossily, so that number is not reused.
        names.push(
            entry
                .map_err(io_error)?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    next(&ordered(names.iter().map(String::as_str))?, title)
}

#[cfg(test)]
mod tests;
