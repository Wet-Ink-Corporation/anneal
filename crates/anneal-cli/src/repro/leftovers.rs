//! Debug instrumentation left behind: lines holding a `[DEBUG-<id>]` tag.
//!
//! Diagnose mode tags every temporary log line, print or probe with `[DEBUG-<id>]` so that one
//! scan finds them all before the work is done. [`debug_leftovers`] is that scan: it reads the
//! files git knows of under a directory (tracked, plus untracked files git does not ignore),
//! prose included, since instrumentation can sit in any file. The Stop hook (#26) calls it, and
//! `anneal repro leftovers` prints it.
//!
//! A tag is `[DEBUG-`, an id of 1 to 64 ASCII letters, digits, `_` or `-`, then `]`; so text
//! that names the pattern, as `[DEBUG-<id>]`, is not a leftover.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::paths::{self, PathError};

/// How long `git ls-files` may take.
const GIT_TIMEOUT: Duration = Duration::from_secs(60);
/// Most bytes read from `git ls-files`; more is an error, never a partial scan.
const MAX_LISTING_BYTES: u64 = 16 * 1024 * 1024;
/// Most bytes read from one file; a larger file is listed in [`LeftoverScan::skipped`].
pub(crate) const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Most bytes read from all files of one scan; past it, files are listed in
/// [`LeftoverScan::skipped`] and [`LeftoverScan::over_budget`] is set.
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Most leftovers reported; the rest are counted in [`LeftoverScan::omitted`].
const MAX_LEFTOVERS: usize = 1000;

/// What opens a tag.
const TAG_OPEN: &str = "[DEBUG-";
/// The longest id a tag may carry.
const MAX_ID_CHARS: usize = 64;

/// One line holding a `[DEBUG-<id>]` tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leftover {
    file: String,
    line: usize,
}

impl Leftover {
    /// The file, relative to the scanned directory, `/`-separated as git prints it (a name that
    /// is not UTF-8 is shown with U+FFFD in place of its bad bytes).
    #[must_use]
    pub fn file(&self) -> &str {
        &self.file
    }

    /// The line, from 1.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }
}

/// The result of a scan.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct LeftoverScan {
    leftovers: Vec<Leftover>,
    omitted: u64,
    skipped: Vec<String>,
    over_budget: bool,
}

/// How much one scan reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Most bytes read from one file.
    pub(crate) file_bytes: u64,
    /// Most bytes read from all files together.
    pub(crate) total_bytes: u64,
}

/// The limits of [`debug_leftovers`].
pub(crate) const LIMITS: Limits = Limits {
    file_bytes: MAX_FILE_BYTES,
    total_bytes: MAX_TOTAL_BYTES,
};

impl LeftoverScan {
    /// The tagged lines, by file then line; at most 1,000.
    #[must_use]
    pub fn leftovers(&self) -> &[Leftover] {
        &self.leftovers
    }

    /// Tagged lines found past the first 1,000.
    #[must_use]
    pub const fn omitted(&self) -> u64 {
        self.omitted
    }

    /// Files not read, so not known to be clean: over 1 MiB, past the total budget, or (outside
    /// unix) named in bytes that are not UTF-8.
    #[must_use]
    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }

    /// True when the total byte budget ran out, so some files were skipped for it.
    #[must_use]
    pub const fn over_budget(&self) -> bool {
        self.over_budget
    }

    /// True when no tagged line was found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.leftovers.is_empty() && self.omitted == 0
    }

    fn push(&mut self, file: &str, line: usize) {
        if self.leftovers.len() < MAX_LEFTOVERS {
            self.leftovers.push(Leftover {
                // The finding owns the path it names.
                file: file.to_owned(),
                line,
            });
        } else {
            self.omitted = self.omitted.saturating_add(1);
        }
    }
}

/// Why the scan could not finish.
#[derive(Debug, thiserror::Error)]
pub enum LeftoverError {
    /// git could not run, did not answer in time, or failed (outside a repository too).
    #[error("listing the files git knows of")]
    Git(#[source] PathError),
    /// `git ls-files` printed more than the scan reads.
    #[error("git ls-files printed more than {max} bytes")]
    TooLarge {
        /// The limit, in bytes.
        max: u64,
    },
    /// A file git listed could not be read.
    #[error("reading {}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

/// Every line holding a `[DEBUG-<id>]` tag in the files git knows of under `root`.
///
/// Those are the tracked files and the untracked ones git does not ignore, each read up to
/// 1 MiB and 64 MiB in all; a file over either is listed in [`LeftoverScan::skipped`] (and
/// running out of the total sets [`LeftoverScan::over_budget`]), as is, outside unix, a file
/// whose name is not UTF-8. Files listed but absent, and anything that is not a regular file
/// (a symlink, a submodule), are passed over.
///
/// # Errors
/// [`LeftoverError::Git`] when git cannot list the files (outside a repository too),
/// [`LeftoverError::TooLarge`] when the listing is over 16 MiB, [`LeftoverError::Read`] when a
/// listed file cannot be read.
pub fn debug_leftovers(root: &Path) -> Result<LeftoverScan, LeftoverError> {
    scan(root, LIMITS)
}

/// [`debug_leftovers`] within `limits`.
pub(crate) fn scan(root: &Path, limits: Limits) -> Result<LeftoverScan, LeftoverError> {
    scan_within(root, limits, GIT_TIMEOUT)
}

/// [`scan`], with `git ls-files` given `git_timeout`: for a caller with a deadline, such as the
/// Stop hook.
pub(crate) fn scan_within(
    root: &Path,
    limits: Limits,
    git_timeout: Duration,
) -> Result<LeftoverScan, LeftoverError> {
    let listing = listing(root, git_timeout)?;
    let mut found = LeftoverScan::default();
    let mut remaining = limits.total_bytes;
    for raw in names(&listing) {
        // Reports name the file in UTF-8; the file itself is opened by its raw name.
        let name = String::from_utf8_lossy(raw);
        #[cfg(unix)]
        let relative = path_of(raw);
        #[cfg(not(unix))]
        let Some(relative) = path_of(raw) else {
            found.skipped.push(name.into_owned());
            continue;
        };
        let cap = Cap {
            file: limits.file_bytes,
            remaining,
        };
        match read_capped(&root.join(relative), cap)? {
            Contents::Absent => {}
            Contents::TooLarge => found.skipped.push(name.into_owned()),
            Contents::OverBudget => {
                found.over_budget = true;
                found.skipped.push(name.into_owned());
            }
            Contents::Bytes(bytes) => {
                let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                remaining = remaining.saturating_sub(read);
                for (line, content) in (1_usize..).zip(bytes.split(|byte| *byte == b'\n')) {
                    if tagged(content) {
                        found.push(&name, line);
                    }
                }
            }
        }
    }
    Ok(found)
}

/// True when `line` holds a `[DEBUG-<id>]` tag. It reads bytes, so a file that is not UTF-8
/// is scanned as it is.
pub(crate) fn tagged(line: impl AsRef<[u8]>) -> bool {
    let line = line.as_ref();
    let open = TAG_OPEN.as_bytes();
    line.windows(open.len())
        .enumerate()
        .filter(|(_, window)| *window == open)
        .any(|(start, _)| {
            let rest = line.get(start.saturating_add(open.len())..).unwrap_or(&[]);
            let id_len = rest
                .iter()
                .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                .count();
            (1..=MAX_ID_CHARS).contains(&id_len) && rest.get(id_len) == Some(&b']')
        })
}

/// The file names in a `git ls-files -z` listing, sorted by bytes and without repeats (an
/// unmerged file is listed once per stage).
fn names(listing: &[u8]) -> Vec<&[u8]> {
    let mut names: Vec<&[u8]> = listing
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The relative path a raw file name from git names. On unix any bytes are a path.
#[cfg(unix)]
fn path_of(raw: &[u8]) -> &Path {
    use std::os::unix::ffi::OsStrExt;
    Path::new(std::ffi::OsStr::from_bytes(raw))
}

/// The relative path a raw file name from git names: git prints names in UTF-8 here, so other
/// bytes name no file this process can open, and the file is skipped.
#[cfg(not(unix))]
fn path_of(raw: &[u8]) -> Option<&Path> {
    std::str::from_utf8(raw).map(Path::new).ok()
}

/// The `git ls-files -z` listing of the files git knows of under `root`, relative to it.
fn listing(root: &Path, git_timeout: Duration) -> Result<Vec<u8>, LeftoverError> {
    let args = [
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "-z",
    ];
    let output = paths::run_git_capped(
        root,
        &args,
        git_timeout,
        MAX_LISTING_BYTES.saturating_add(1),
    )
    .map_err(LeftoverError::Git)?;
    if !output.status.success() {
        return Err(LeftoverError::Git(paths::git_failure(root, &output.stderr)));
    }
    if u64::try_from(output.stdout.len()).map_or(true, |len| len > MAX_LISTING_BYTES) {
        return Err(LeftoverError::TooLarge {
            max: MAX_LISTING_BYTES,
        });
    }
    Ok(output.stdout)
}

/// How much one file may take: its own cap, and what is left of the scan's total.
#[derive(Debug, Clone, Copy)]
struct Cap {
    file: u64,
    remaining: u64,
}

/// A listed file as read.
enum Contents {
    /// Not there, or not a regular file.
    Absent,
    /// Over the per-file cap.
    TooLarge,
    /// Within the per-file cap, but over what is left of the total.
    OverBudget,
    /// Its bytes.
    Bytes(Vec<u8>),
}

/// Reads `path` within `cap`, checking first that it is a regular file (a pipe would block, a
/// directory fails on Windows) and that its size fits, so a file that does not is never read.
/// One byte past the cap is read, so a file that grew since its size was taken still counts as
/// too large rather than being cut short.
fn read_capped(path: &Path, cap: Cap) -> Result<Contents, LeftoverError> {
    let failed = |source: io::Error| LeftoverError::Read {
        path: path.to_path_buf(),
        source,
    };
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Contents::Absent),
        Err(error) => return Err(failed(error)),
    };
    if !metadata.is_file() {
        return Ok(Contents::Absent);
    }
    let judge = |len: u64| {
        if len > cap.file {
            Some(Contents::TooLarge)
        } else if len > cap.remaining {
            Some(Contents::OverBudget)
        } else {
            None
        }
    };
    if let Some(refused) = judge(metadata.len()) {
        return Ok(refused);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(failed)?
        .take(cap.file.min(cap.remaining).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    match judge(u64::try_from(bytes.len()).unwrap_or(u64::MAX)) {
        Some(refused) => Ok(refused),
        None => Ok(Contents::Bytes(bytes)),
    }
}

#[cfg(test)]
mod tests;
