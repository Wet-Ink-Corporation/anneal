//! The out-of-scope ruling store: one `.out-of-scope/<slug>.md` file per ruling.
//!
//! Each ruling opens with frontmatter in the same restricted YAML as a `SKILL.md`
//! (`lint::skills::frontmatter`): `id`, `ruled_at` and `summary` are required, `weigh_in_ref`
//! is optional, and no other key is allowed. [`list`] returns every ruling sorted by id, so
//! D6's `choice` options come out in the same order on every run. A ruling that does not read
//! is an error naming its file, never skipped. See `docs/define.md`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::lint::skills::discover::{Capped, read_bounded};
use crate::lint::skills::frontmatter::{self, Field};

/// The store's directory, relative to the repository root.
pub const STORE_DIR: &str = ".out-of-scope";

/// The frontmatter keys a ruling may have.
const KEYS: [&str; 4] = ["id", "ruled_at", "summary", "weigh_in_ref"];

/// One valid ruling. [`parse`] (and so [`list`]) is the only way to build one, so every
/// `Ruling` has passed the field checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruling {
    id: String,
    ruled_at: String,
    summary: String,
    weigh_in_ref: Option<String>,
    file: PathBuf,
}

impl Ruling {
    /// Stable id: one word of printable ASCII, unique in the store.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// When it was ruled: `YYYY-MM-DD`, optionally followed by `T` and a time.
    #[must_use]
    pub fn ruled_at(&self) -> &str {
        &self.ruled_at
    }

    /// One line saying what is out of scope.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// The weigh-in that decided it, if any.
    #[must_use]
    pub fn weigh_in_ref(&self) -> Option<&str> {
        self.weigh_in_ref.as_deref()
    }

    /// The file it was read from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }
}

/// Why the rulings could not be read. Every variant names the file (or directory) at fault.
#[derive(Debug, thiserror::Error)]
pub enum RulingError {
    /// The store or a ruling could not be read.
    #[error("{path}: reading failed", path = path.display())]
    Io {
        /// The directory or file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// A ruling is larger than the read cap.
    #[error("{file}: {size} bytes is over the read cap", file = file.display())]
    TooLarge {
        /// The ruling file.
        file: PathBuf,
        /// Its size in bytes.
        size: u64,
    },
    /// A ruling is not UTF-8.
    #[error("{file}: not UTF-8", file = file.display())]
    NotUtf8 {
        /// The ruling file.
        file: PathBuf,
    },
    /// The frontmatter does not parse.
    #[error("{file}:{line}: {reason}", file = file.display())]
    Frontmatter {
        /// The ruling file.
        file: PathBuf,
        /// 1-based line where parsing stopped.
        line: usize,
        /// What was wrong.
        reason: &'static str,
    },
    /// A required key is absent.
    #[error("{file}: no `{field}` key", file = file.display())]
    MissingField {
        /// The ruling file.
        file: PathBuf,
        /// The missing key.
        field: &'static str,
    },
    /// A key a ruling does not have.
    #[error(
        "{file}:{line}: unknown key `{field}` (a ruling has id, ruled_at, summary, weigh_in_ref)",
        file = file.display()
    )]
    UnknownField {
        /// The ruling file.
        file: PathBuf,
        /// 1-based line of the key.
        line: usize,
        /// The key as written.
        field: String,
    },
    /// A key's value is not valid.
    #[error("{file}:{line}: `{field}` {reason}", file = file.display())]
    InvalidField {
        /// The ruling file.
        file: PathBuf,
        /// 1-based line of the key.
        line: usize,
        /// The key.
        field: &'static str,
        /// What a valid value looks like.
        reason: &'static str,
    },
    /// The store holds more than [`list_within`] looks at.
    #[error("{dir}: more than {limit} {what}", dir = dir.display())]
    TooMany {
        /// The store's directory.
        dir: PathBuf,
        /// The limit passed.
        limit: usize,
        /// What was counted: `directory entries` or `ruling files`.
        what: &'static str,
    },
    /// Two rulings share an id.
    #[error(
        "{second}: id `{id}` is already used by {first}",
        first = first.display(),
        second = second.display()
    )]
    DuplicateId {
        /// The shared id.
        id: String,
        /// The file that used it first (in file-name order).
        first: PathBuf,
        /// The file that used it again.
        second: PathBuf,
    },
}

/// Reads the ruling in `text`, the contents of `file`. Only the frontmatter is read; the body
/// is free prose.
///
/// # Errors
/// [`RulingError::Frontmatter`] when the frontmatter does not parse,
/// [`RulingError::MissingField`], [`RulingError::UnknownField`] or
/// [`RulingError::InvalidField`] when its keys are wrong.
pub fn parse(file: &Path, text: &str) -> Result<Ruling, RulingError> {
    let frontmatter_error = |line, reason| RulingError::Frontmatter {
        file: file.to_path_buf(),
        line,
        reason,
    };
    let opening = text.lines().next().unwrap_or("");
    if opening.trim_end() != "---" {
        return Err(frontmatter_error(
            1,
            "a ruling must start with a `---` frontmatter line",
        ));
    }
    let doc = frontmatter::parse(text).map_err(|e| frontmatter_error(e.line, e.reason))?;
    if let Some(unknown) = doc.fields.iter().find(|f| !KEYS.contains(&f.key.as_str())) {
        return Err(RulingError::UnknownField {
            file: file.to_path_buf(),
            line: unknown.line,
            field: unknown.key.clone(),
        });
    }
    let mut fields = Fields {
        file,
        fields: doc.fields,
    };
    Ok(Ruling {
        id: fields.required("id", valid_id, "must be one word of printable ASCII")?,
        ruled_at: fields.required(
            "ruled_at",
            valid_date,
            "must be a date, YYYY-MM-DD, optionally followed by T and a time",
        )?,
        summary: fields.required("summary", valid_summary, "must be one non-empty line")?,
        weigh_in_ref: fields.optional("weigh_in_ref", valid_id, "must be one word when present")?,
        file: file.to_path_buf(),
    })
}

/// The fields of one ruling's frontmatter, taken out one key at a time.
struct Fields<'a> {
    file: &'a Path,
    fields: Vec<Field>,
}

impl Fields<'_> {
    fn required(
        &mut self,
        key: &'static str,
        valid: fn(&str) -> bool,
        reason: &'static str,
    ) -> Result<String, RulingError> {
        self.optional(key, valid, reason)?
            .ok_or_else(|| RulingError::MissingField {
                file: self.file.to_path_buf(),
                field: key,
            })
    }

    fn optional(
        &mut self,
        key: &'static str,
        valid: fn(&str) -> bool,
        reason: &'static str,
    ) -> Result<Option<String>, RulingError> {
        let Some(index) = self.fields.iter().position(|f| f.key == key) else {
            return Ok(None);
        };
        let Field { value, line, .. } = self.fields.swap_remove(index);
        if !valid(&value) {
            return Err(RulingError::InvalidField {
                file: self.file.to_path_buf(),
                line,
                field: key,
                reason,
            });
        }
        Ok(Some(value))
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_graphic())
}

fn valid_summary(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains('\n')
}

/// `YYYY-MM-DD` (month 01 to 12, day 01 to 31), optionally followed by `T` and anything.
fn valid_date(value: &str) -> bool {
    let (date, rest) = value.split_at_checked(10).unwrap_or((value, ""));
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let in_range = |s: &str, max: u8| s.parse::<u8>().is_ok_and(|v| (1..=max).contains(&v));
    digits(year, 4)
        && digits(month, 2)
        && digits(day, 2)
        && in_range(month, 12)
        && in_range(day, 31)
        && (rest.is_empty() || rest.starts_with('T'))
}

/// Every ruling in `dir` (normally [`STORE_DIR`]), sorted by id in byte order.
///
/// Every regular `*.md` file (extension in any letter case) is a ruling; other entries are not
/// read. A missing directory has no rulings.
///
/// # Errors
/// [`RulingError::Io`] when the directory or a ruling cannot be read,
/// [`RulingError::TooLarge`] or [`RulingError::NotUtf8`] for an unreadable ruling, the errors
/// of [`parse`] for an invalid one, and [`RulingError::DuplicateId`] when two share an id.
pub fn list(dir: &Path) -> Result<Vec<Ruling>, RulingError> {
    from_files(ruling_files(dir)?)
}

/// How much of a store [`list_within`] looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListLimits {
    /// Most directory entries read, of any kind.
    pub max_entries: usize,
    /// Most ruling files read.
    pub max_files: usize,
}

/// As [`list`], within `limits`, for a caller with a time budget (a hook).
///
/// At most `limits.max_entries` directory entries and `limits.max_files` ruling files are
/// looked at, and a ruling is only a regular file by its own entry (`DirEntry::file_type`), so
/// a symlink is never followed.
///
/// # Errors
/// As [`list`], and [`RulingError::TooMany`] when the store holds more entries or ruling files
/// than `limits` allow: the list fails rather than being cut short.
pub fn list_within(dir: &Path, limits: ListLimits) -> Result<Vec<Ruling>, RulingError> {
    from_files(bounded_ruling_files(dir, limits)?)
}

/// The rulings in `files`, read, parsed, sorted by id and checked for duplicate ids.
fn from_files(files: Vec<PathBuf>) -> Result<Vec<Ruling>, RulingError> {
    let mut rulings = Vec::with_capacity(files.len());
    for file in files {
        let text = read(&file)?;
        rulings.push(parse(&file, &text)?);
    }
    // Stable: rulings sharing an id stay in file-name order, so `first` is the earlier file.
    rulings.sort_by(|a, b| a.id.cmp(&b.id));
    let duplicate = rulings.windows(2).find_map(|pair| match pair {
        [first, second] if first.id == second.id => Some((first, second)),
        _ => None,
    });
    if let Some((first, second)) = duplicate {
        // The error outlives `rulings`, so it owns copies of the id and both paths.
        return Err(RulingError::DuplicateId {
            id: second.id.clone(),
            first: first.file.clone(),
            second: second.file.clone(),
        });
    }
    Ok(rulings)
}

/// The `*.md` regular files in `dir`, sorted by name, so a duplicate id is always reported
/// against the same first file.
fn ruling_files(dir: &Path) -> Result<Vec<PathBuf>, RulingError> {
    let io_error = |source| RulingError::Io {
        path: dir.to_path_buf(),
        source,
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.map_err(io_error)?.path();
        let markdown = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        if markdown && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// As [`ruling_files`], within `limits` and without following symlinks.
fn bounded_ruling_files(dir: &Path, limits: ListLimits) -> Result<Vec<PathBuf>, RulingError> {
    let io_error = |source| RulingError::Io {
        path: dir.to_path_buf(),
        source,
    };
    let too_many = |limit, what| RulingError::TooMany {
        dir: dir.to_path_buf(),
        limit,
        what,
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    let mut files = Vec::new();
    // Only the entry after the limit fails the list, so a store exactly at it still lists.
    for (seen, entry) in entries.enumerate() {
        if seen >= limits.max_entries {
            return Err(too_many(limits.max_entries, "directory entries"));
        }
        let entry = entry.map_err(io_error)?;
        let markdown = Path::new(&entry.file_name())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        // `file_type` describes the entry itself: a symlink is not a regular file.
        if markdown && entry.file_type().map_err(io_error)?.is_file() {
            if files.len() >= limits.max_files {
                return Err(too_many(limits.max_files, "ruling files"));
            }
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

fn read(file: &Path) -> Result<String, RulingError> {
    let Capped { size, bytes } = read_bounded(file).map_err(|source| RulingError::Io {
        path: file.to_path_buf(),
        source,
    })?;
    let bytes = bytes.ok_or_else(|| RulingError::TooLarge {
        file: file.to_path_buf(),
        size,
    })?;
    String::from_utf8(bytes).map_err(|_not_utf8| RulingError::NotUtf8 {
        file: file.to_path_buf(),
    })
}

#[cfg(test)]
mod tests;
