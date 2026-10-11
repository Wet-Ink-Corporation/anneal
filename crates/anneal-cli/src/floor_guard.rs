//! `anneal floor-guard [--base <ref>]`: refuse a change that lowers a quality floor.
//!
//! - `diff`: reads `git diff -U0` output into files, hunks and numbered lines.
//! - `detect`: turns them into findings: suppressions added, test files deleted or emptied,
//!   more tests removed than added, and coverage, timeout and ratchet thresholds loosened.
//! - `git`: the git calls and working-tree reads, each bounded.
//!
//! The diff is the working tree (committed and uncommitted changes) against the merge base of
//! `HEAD` and the base (`--base`, else `origin/main`, else `main`), plus every untracked file
//! git does not ignore, read as all added. Exit 0 when no floor was lowered, 1 when one was
//! (one finding per hit in `data.findings`), and 2 with one `ERROR` line when the check could
//! not run: not in a repository, a base that does not resolve, a git failure or timeout, or
//! output over its cap. It complements temper's escape-hatch scan rather than replacing it;
//! `docs/floor-guard.md` describes the split.

mod detect;
mod diff;
mod git;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths::PathError;
pub use detect::{Category, FloorFinding};
use detect::{Detector, PostImage};
use diff::{FileDiff, Status};
use git::FileRead;

/// `anneal floor-guard`.
#[derive(Debug, Args)]
pub struct FloorGuardArgs {
    /// The base to diff against (through its merge base with HEAD); default origin/main, else
    /// main
    #[arg(long, value_name = "REF")]
    base: Option<String>,
}

/// Why `anneal floor-guard` could not check the diff. Every variant exits 2: a check that did
/// not run never reads as 0.
#[derive(Debug, thiserror::Error)]
pub enum FloorError {
    /// git could not run, timed out, or said this is not a repository.
    #[error("floor-guard")]
    Git(#[source] PathError),
    /// `--base` names no commit.
    #[error("floor-guard: base {base} does not resolve to a commit")]
    BaseNotFound {
        /// The ref as given.
        base: String,
    },
    /// No `--base`, and neither default resolves.
    #[error("floor-guard: no base: neither origin/main nor main resolves; pass --base <ref>")]
    NoDefaultBase,
    /// `HEAD` and the base share no history (or there is no `HEAD` yet).
    #[error("floor-guard: HEAD and {base} have no merge base: {stderr}")]
    NoMergeBase {
        /// The base ref.
        base: String,
        /// What git said.
        stderr: String,
    },
    /// A git call exited non-zero.
    #[error("floor-guard: git {what} failed: {stderr}")]
    GitFailed {
        /// The git subcommand.
        what: &'static str,
        /// What git said.
        stderr: String,
    },
    /// A git call printed more than its cap; the check refuses to judge part of the diff.
    #[error("floor-guard: git {what} printed more than {max} bytes; refusing a partial check")]
    TooLarge {
        /// The git subcommand.
        what: &'static str,
        /// The cap.
        max: u64,
    },
    /// A working-tree file could not be read.
    #[error("floor-guard: reading {}", path.display())]
    Read {
        /// The path, relative to the repository root.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

impl FloorError {
    /// The documented exit code: always 2 (`tool error`). Exhaustive, so a new variant has to
    /// choose.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Git(_)
            | Self::BaseNotFound { .. }
            | Self::NoDefaultBase
            | Self::NoMergeBase { .. }
            | Self::GitFailed { .. }
            | Self::TooLarge { .. }
            | Self::Read { .. } => exit_codes::FLOOR_TOOL_ERROR,
        }
    }
}

/// A file the check could not read whole, and why; listed rather than dropped.
#[derive(Debug, Serialize)]
struct Skipped {
    file: String,
    reason: &'static str,
}

#[derive(Debug, Serialize)]
struct Detail<'a> {
    key: String,
    #[serde(flatten)]
    finding: &'a FloorFinding,
}

#[derive(Debug, Serialize)]
struct Data<'a> {
    base: &'a str,
    base_commit: &'a str,
    merge_base: &'a str,
    files: usize,
    untracked: usize,
    findings: Vec<Detail<'a>>,
    /// Findings past `detect::MAX_FINDINGS`, counted rather than listed.
    omitted: usize,
    skipped: Vec<Skipped>,
}

/// Runs `anneal floor-guard` in `cwd`.
pub(crate) fn run<O: Write>(
    args: &FloorGuardArgs,
    cwd: &Path,
    out: &mut O,
) -> Result<Code, CliError> {
    let root = git::toplevel(cwd)?;
    let (base, base_commit) = resolve_base(&root, args.base.as_deref())?;
    let merge_base = git::merge_base(&root, base, &base_commit)?;
    let text = git::diff(&root, &merge_base)?;
    let files = diff::parse(&text);
    let mut detector = Detector::default();
    let mut skipped = Vec::new();
    for file in &files {
        let post = post_image(&root, file, &mut skipped)?;
        detector.file(file, post.as_ref());
    }
    let untracked = git::untracked(&root)?;
    let untracked_count = untracked.len();
    for path in untracked {
        match git::read_file(&root, &path)? {
            FileRead::Text(content) => detector.file(&FileDiff::untracked(&path, &content), None),
            FileRead::TooLarge => skipped.push(Skipped {
                file: path,
                reason: "untracked and over 1 MiB: not scanned",
            }),
            FileRead::Absent => {}
        }
    }
    let report = detector.finish();
    let code = if report.findings.is_empty() && report.omitted == 0 {
        exit_codes::FLOOR_HELD
    } else {
        exit_codes::FLOOR_LOWERED
    };
    let data = Data {
        base,
        base_commit: &base_commit,
        merge_base: &merge_base,
        files: files.len(),
        untracked: untracked_count,
        findings: report
            .findings
            .iter()
            .map(|finding| Detail {
                key: finding.key(),
                finding,
            })
            .collect(),
        omitted: report.omitted,
        skipped,
    };
    let findings: Vec<Finding> = report
        .findings
        .iter()
        .map(|finding| Finding::new(finding.key(), finding.message.as_str()))
        .collect();
    Envelope::new(CommandName::FloorGuard, None, code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// The base ref and its commit: `--base` when given, else the first of
/// [`git::DEFAULT_BASES`] that resolves.
fn resolve_base<'a>(root: &Path, base: Option<&'a str>) -> Result<(&'a str, String), FloorError> {
    if let Some(base) = base {
        return match git::resolve(root, base)? {
            Some(commit) => Ok((base, commit)),
            None => Err(FloorError::BaseNotFound {
                // The error owns the ref it names.
                base: base.to_owned(),
            }),
        };
    }
    for base in git::DEFAULT_BASES {
        if let Some(commit) = git::resolve(root, base)? {
            return Ok((base, commit));
        }
    }
    Err(FloorError::NoDefaultBase)
}

/// What the detector needs of `file`'s working-tree content; `None` for a deleted file or one
/// that is not a regular file. One over the read cap is listed in `skipped`.
fn post_image(
    root: &Path,
    file: &FileDiff<'_>,
    skipped: &mut Vec<Skipped>,
) -> Result<Option<PostImage>, FloorError> {
    if file.status == Status::Deleted {
        return Ok(None);
    }
    match git::read_file(root, &file.path)? {
        FileRead::Text(content) => Ok(Some(PostImage::of(&content))),
        FileRead::TooLarge => {
            skipped.push(Skipped {
                // The listing owns the paths it reports.
                file: file.path.as_ref().to_owned(),
                reason: "over 1 MiB: the emptied-test and coverageThreshold checks skipped",
            });
            Ok(None)
        }
        FileRead::Absent => Ok(None),
    }
}
