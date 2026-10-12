//! The working tree's change, file by file, as `anneal verify risk` reads it: the same diff
//! `floor-guard` judges (committed and uncommitted changes against the merge base of `HEAD` and
//! the base, renames found) plus every untracked file git does not ignore, read as all added.
//! Every call is one of `floor-guard`'s own, so the bounds are its: 60 s per git call, 16 MiB
//! of diff or listing, 1 MiB per untracked file.

use std::path::Path;

use super::diff::{self, FileDiff};
use super::git::{self, FileRead};
use super::{FloorError, resolve_base};

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Touched {
    /// Its path in the working tree (for a deleted file, the path it had), `/`-separated.
    pub(crate) path: String,
    /// The path it was renamed from, when it was.
    pub(crate) old_path: Option<String>,
    /// Lines added plus removed; `None` for an untracked file over the read cap. A binary
    /// file's diff has no lines, so it counts none.
    pub(crate) lines: Option<u64>,
}

/// The working tree's change against a base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkingChange {
    /// The base, as given or defaulted.
    pub(crate) base: String,
    /// The merge base of `HEAD` and the base.
    pub(crate) merge_base: String,
    /// The tracked files in diff order, then the untracked ones.
    pub(crate) files: Vec<Touched>,
}

/// The change of the worktree holding `cwd` against `base` (else `origin/main`, else `main`).
///
/// # Errors
/// The [`FloorError`] `floor-guard` would give: not a repository, a base that does not
/// resolve, no merge base, a git failure or timeout, output over its cap, an unreadable file.
pub(crate) fn working_change(cwd: &Path, base: Option<&str>) -> Result<WorkingChange, FloorError> {
    let root = git::toplevel(cwd)?;
    let (base, base_commit) = resolve_base(&root, base)?;
    let merge_base = git::merge_base(&root, base, &base_commit)?;
    let text = git::diff(&root, &merge_base)?;
    let mut files: Vec<Touched> = diff::parse(&text).iter().map(touched).collect();
    for path in git::untracked(&root)? {
        let lines = match git::read_file(&root, &path)? {
            FileRead::Text(content) => Some(line_count(&FileDiff::untracked(&path, &content))),
            FileRead::TooLarge => None,
            // Gone since git listed it, or not a regular file: nothing of it to count.
            FileRead::Absent => Some(0),
        };
        files.push(Touched {
            path,
            old_path: None,
            lines,
        });
    }
    Ok(WorkingChange {
        // The change owns the base it names.
        base: base.to_owned(),
        merge_base,
        files,
    })
}

/// `file` as a touched file.
fn touched(file: &FileDiff<'_>) -> Touched {
    Touched {
        // The change outlives the diff text the paths borrow from.
        path: file.path.as_ref().to_owned(),
        old_path: (file.old_path != file.path).then(|| file.old_path.as_ref().to_owned()),
        lines: Some(line_count(file)),
    }
}

/// Lines added plus removed in `file`'s hunks.
fn line_count(file: &FileDiff<'_>) -> u64 {
    file.hunks
        .iter()
        .map(|hunk| hunk.added.len().saturating_add(hunk.removed.len()))
        .map(|lines| u64::try_from(lines).unwrap_or(u64::MAX))
        .fold(0, u64::saturating_add)
}

#[cfg(test)]
mod tests;
