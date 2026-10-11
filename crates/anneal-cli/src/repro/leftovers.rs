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
/// Most leftovers reported; the rest are counted in [`LeftoverScan::omitted`].
const MAX_LEFTOVERS: usize = 1000;

/// What opens a tag.
const TAG_OPEN: &str = "[DEBUG-";
/// The longest id a tag may carry.
const MAX_ID_CHARS: usize = 64;

/// One line holding a `[DEBUG-<id>]` tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leftover {
    /// The file, relative to the scanned directory, `/`-separated as git prints it.
    pub file: String,
    /// The line, from 1.
    pub line: usize,
}

/// The result of a scan.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct LeftoverScan {
    /// The tagged lines, by file then line; at most 1,000.
    pub leftovers: Vec<Leftover>,
    /// Tagged lines found past the first 1,000.
    pub omitted: u64,
    /// Files over 1 MiB, not read, so not known to be clean.
    pub skipped: Vec<String>,
}

impl LeftoverScan {
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
/// 1 MiB (larger ones are listed in [`LeftoverScan::skipped`]). Files listed but absent, and
/// anything that is not a regular file (a symlink, a submodule), are passed over.
///
/// # Errors
/// [`LeftoverError::Git`] when git cannot list the files (outside a repository too),
/// [`LeftoverError::TooLarge`] when the listing is over 16 MiB, [`LeftoverError::Read`] when a
/// listed file cannot be read.
pub fn debug_leftovers(root: &Path) -> Result<LeftoverScan, LeftoverError> {
    let mut found = LeftoverScan::default();
    for file in listed(root)? {
        match read_capped(&root.join(&file))? {
            Contents::Absent => {}
            Contents::TooLarge => found.skipped.push(file),
            Contents::Text(text) => {
                for (line, content) in (1_usize..).zip(text.lines()) {
                    if tagged(content) {
                        found.push(&file, line);
                    }
                }
            }
        }
    }
    Ok(found)
}

/// True when `line` holds a `[DEBUG-<id>]` tag.
pub(crate) fn tagged(line: &str) -> bool {
    line.match_indices(TAG_OPEN).any(|(start, open)| {
        let rest = line.get(start.saturating_add(open.len())..).unwrap_or("");
        let id_len = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            .count();
        (1..=MAX_ID_CHARS).contains(&id_len) && rest.as_bytes().get(id_len) == Some(&b']')
    })
}

/// The files git knows of under `root`, relative to it, sorted and without repeats (an
/// unmerged file is listed once per stage).
fn listed(root: &Path) -> Result<Vec<String>, LeftoverError> {
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
        GIT_TIMEOUT,
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
    let mut files: Vec<String> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect();
    files.sort_unstable();
    files.dedup();
    Ok(files)
}

/// A listed file as read.
enum Contents {
    /// Not there, or not a regular file.
    Absent,
    /// Over [`MAX_FILE_BYTES`].
    TooLarge,
    /// Its content, as lossy UTF-8.
    Text(String),
}

/// Reads `path` up to [`MAX_FILE_BYTES`], checking first that it is a regular file (a pipe
/// would block, a directory fails on Windows).
fn read_capped(path: &Path) -> Result<Contents, LeftoverError> {
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
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(failed)?
        .take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_FILE_BYTES) {
        return Ok(Contents::TooLarge);
    }
    Ok(Contents::Text(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests;
