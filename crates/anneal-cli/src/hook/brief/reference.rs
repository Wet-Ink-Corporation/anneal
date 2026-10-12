//! The brief an Agent call's prompt names: one `Brief: <path>` line, the path relative to the
//! worktree root and under `.anneal/briefs/`, resolved (symlinks included) to a file inside
//! that directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::brief::BRIEFS_DIR;

/// What starts the line naming the brief.
const MARKER: &str = "Brief:";

/// Why the prompt names no usable brief.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// No `Brief:` line.
    None,
    /// More than one `Brief:` line.
    Several,
    /// The path is absolute, has `\`, `.` or `..` parts, or is not under `.anneal/briefs/`.
    NotUnderBriefs(String),
    /// Nothing is at the path.
    Missing(String),
    /// The path resolves (through a symlink) outside `.anneal/briefs/` of the worktree.
    Escapes(String),
    /// The path could not be resolved.
    Unresolvable(String, String),
}

impl Refusal {
    /// The reason, in one line.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::None => format!(
                "the prompt names no brief; write one with `anneal brief build --ticket <id> \
                 --base <ref>` and add a line `{MARKER} {BRIEFS_DIR}/<id>.md` to the prompt"
            ),
            Self::Several => format!("the prompt has more than one `{MARKER}` line; keep one"),
            Self::NotUnderBriefs(path) => format!(
                "the brief {path:?} is not a relative path under {BRIEFS_DIR}/ (no `..`, `.`, \
                 `\\` or absolute paths)"
            ),
            Self::Missing(path) => {
                format!("the brief {path} does not exist; write it with `anneal brief build`")
            }
            Self::Escapes(path) => {
                format!("the brief {path} resolves outside {BRIEFS_DIR}/ of this worktree")
            }
            Self::Unresolvable(path, why) => format!("the brief {path} cannot be resolved: {why}"),
        }
    }
}

/// The path text of the one `Brief:` line in `prompt`, trimmed and without surrounding
/// backticks.
pub(crate) fn find(prompt: &str) -> Result<&str, Refusal> {
    let mut named = prompt
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix(MARKER))
        .map(|rest| {
            let rest = rest.trim();
            rest.strip_prefix('`')
                .and_then(|inner| inner.strip_suffix('`'))
                .unwrap_or(rest)
        });
    match (named.next(), named.next()) {
        (Some(path), None) => Ok(path),
        (None, _) => Err(Refusal::None),
        (Some(_), Some(_)) => Err(Refusal::Several),
    }
}

/// The real path of the brief `text` names, under the worktree `root`.
pub(crate) fn resolve(root: &Path, text: &str) -> Result<PathBuf, Refusal> {
    let refused = || Refusal::NotUnderBriefs(text.to_owned());
    if text.contains('\\') || Path::new(text).is_absolute() {
        return Err(refused());
    }
    let dir_parts: Vec<&str> = BRIEFS_DIR.split('/').collect();
    let parts: Vec<&str> = text.split('/').collect();
    let (head, rest) = parts
        .split_at_checked(dir_parts.len())
        .ok_or_else(refused)?;
    let plain = |part: &&str| !part.is_empty() && *part != "." && *part != "..";
    if head != dir_parts.as_slice() || rest.is_empty() || !rest.iter().all(plain) {
        return Err(refused());
    }
    let dir = dir_parts
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part));
    let canonical = |path: &Path| match fs::canonicalize(path) {
        Ok(real) => Ok(real),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(Refusal::Missing(text.to_owned()))
        }
        Err(error) => Err(Refusal::Unresolvable(text.to_owned(), error.to_string())),
    };
    // The briefs directory itself must be the worktree's own, not a symlink (of `.anneal` or
    // of `briefs`) to somewhere else: compare it with the resolved root's.
    let real_root = canonical(root)?;
    let own_dir = dir_parts
        .iter()
        .fold(real_root, |path, part| path.join(part));
    let real_dir = canonical(&dir)?;
    let file = rest.iter().fold(dir, |path, part| path.join(part));
    let real_file = canonical(&file)?;
    if real_dir == own_dir && real_file.starts_with(&own_dir) {
        Ok(real_file)
    } else {
        Err(Refusal::Escapes(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Refusal, find, resolve};

    #[test]
    fn the_one_brief_line_is_found_anywhere_in_the_prompt_with_or_without_backticks() {
        assert_eq!(
            (
                find("Fix it.\nBrief: .anneal/briefs/36.md\nThanks."),
                find("  Brief: `.anneal/briefs/36.md`  "),
            ),
            (Ok(".anneal/briefs/36.md"), Ok(".anneal/briefs/36.md")),
            "trimmed, backticks dropped"
        );
    }

    #[test]
    fn no_brief_line_or_two_of_them_is_refused() {
        assert_eq!(
            (
                find("no brief here"),
                find("Brief: .anneal/briefs/a.md\nBrief: .anneal/briefs/b.md"),
            ),
            (Err(Refusal::None), Err(Refusal::Several)),
            "exactly one"
        );
    }

    #[test]
    fn paths_outside_the_briefs_directory_are_refused_before_the_disk_is_read() {
        let root = tempfile::tempdir().expect("a directory");
        for text in [
            "",
            ".anneal/briefs/",
            ".anneal/briefs/../x.md",
            ".anneal/briefs/./x.md",
            ".anneal/x.md",
            "briefs/x.md",
            ".anneal\\briefs\\x.md",
            "/etc/passwd",
        ] {
            assert_eq!(
                resolve(root.path(), text),
                Err(Refusal::NotUnderBriefs(text.to_owned())),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_brief_that_is_not_there_is_missing() {
        let root = tempfile::tempdir().expect("a directory");
        std::fs::create_dir_all(root.path().join(".anneal").join("briefs")).expect("created");

        assert_eq!(
            resolve(root.path(), ".anneal/briefs/36.md"),
            Err(Refusal::Missing(".anneal/briefs/36.md".to_owned())),
            "named, not written yet"
        );
    }

    /// A worktree whose `link` (a path under it, `/`-separated) is a symlink to a directory
    /// outside it holding `briefs/36.md` or `36.md`, as `nested` says.
    #[cfg(unix)]
    fn linked_out(link: &str, nested: bool) -> (tempfile::TempDir, tempfile::TempDir) {
        let root = tempfile::tempdir().expect("a worktree");
        let outside = tempfile::tempdir().expect("a directory outside it");
        let target = if nested {
            outside.path().join("briefs")
        } else {
            outside.path().to_path_buf()
        };
        std::fs::create_dir_all(&target).expect("created");
        std::fs::write(target.join("36.md"), "## Goal\nx\n").expect("written");
        let link = link
            .split('/')
            .fold(root.path().to_path_buf(), |path, part| path.join(part));
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent).expect("created");
        }
        std::os::unix::fs::symlink(outside.path(), &link).expect("linked");
        (root, outside)
    }

    #[test]
    #[cfg(unix)]
    fn a_briefs_directory_that_is_a_symlink_out_of_the_worktree_is_refused() {
        let (root, _outside) = linked_out(".anneal/briefs", false);

        assert_eq!(
            resolve(root.path(), ".anneal/briefs/36.md"),
            Err(Refusal::Escapes(".anneal/briefs/36.md".to_owned())),
            "the file is inside the linked directory, but not inside the worktree"
        );
    }

    #[test]
    #[cfg(unix)]
    fn an_anneal_directory_that_is_a_symlink_out_of_the_worktree_is_refused() {
        let (root, _outside) = linked_out(".anneal", true);

        assert_eq!(
            resolve(root.path(), ".anneal/briefs/36.md"),
            Err(Refusal::Escapes(".anneal/briefs/36.md".to_owned())),
            "briefs/ resolves outside the worktree"
        );
    }
}
