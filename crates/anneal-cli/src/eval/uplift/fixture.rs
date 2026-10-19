//! A case's fixture: the directory `<suites>/fixtures/<name>/` whose files seed each of its
//! runs' workspace through a `claude plugin eval` scaffold script.
//!
//! [`FixtureName`] is the name as a case gives it (pure); [`locate`] checks the directory. A
//! fixture may hold relative symlinks into its repository (a `docs` link to the repository's
//! `docs/`, say), which the scaffold copies dereferenced; one that dangles, leads out of the
//! repository or back into a directory it is inside is refused.

use std::cell::OnceCell;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::paths::{self, PathError};

/// The directory under the suites directory that holds the fixtures.
pub(crate) const FIXTURES_DIR: &str = "fixtures";
/// The longest fixture name.
const MAX_NAME: usize = 64;
/// The most entries a fixture may hold, links followed (H-13).
const MAX_ENTRIES: usize = 10_000;
/// How long git may take to name the repository the suites are in.
const GIT_TIMEOUT: Duration = Duration::from_secs(2);

/// A fixture's name: 1 to 64 of `A-Z`, `a-z`, `0-9`, `.`, `_` and `-`, not starting with `.`,
/// so it holds no path separator and is never `.` or `..`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FixtureName(String);

impl FixtureName {
    /// The name as written.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FixtureName {
    type Error = String;

    /// The name in `text`, or `text` back when it is not a fixture name.
    fn try_from(text: String) -> Result<Self, Self::Error> {
        let plain = text.len() <= MAX_NAME
            && text.bytes().next().is_some_and(|first| first != b'.')
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if plain { Ok(Self(text)) } else { Err(text) }
    }
}

impl fmt::Display for FixtureName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A case's fixture, located: its name, and its directory as the absolute path a scaffold
/// copies from, checked by [`locate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fixture {
    /// The name the case gives.
    pub(crate) name: FixtureName,
    /// `<suites>/fixtures/<name>`, absolute.
    pub(crate) dir: String,
}

#[cfg(test)]
impl Fixture {
    /// The fixture `name`, as if located at `/repo/evals/uplift/fixtures/<name>`.
    pub(crate) fn sample(name: &str) -> Self {
        Self {
            name: FixtureName::try_from(name.to_owned()).expect("a fixture name"),
            dir: format!("/repo/evals/uplift/{FIXTURES_DIR}/{name}"),
        }
    }
}

/// The repository a fixture's symlinks must stay in, found on first need: git's top level for
/// the suites directory, else (in no repository) the parent of its nearest `evals` ancestor.
#[derive(Debug)]
pub(crate) struct RepoRoot<'a> {
    suites: &'a Path,
    found: OnceCell<Result<PathBuf, String>>,
}

impl<'a> RepoRoot<'a> {
    /// The root of the repository holding `suites`, not yet looked for.
    pub(crate) const fn new(suites: &'a Path) -> Self {
        Self {
            suites,
            found: OnceCell::new(),
        }
    }

    /// The resolved root, or why there is none.
    fn get(&self) -> Result<&Path, &str> {
        match self.found.get_or_init(|| find_root(self.suites)) {
            Ok(root) => Ok(root),
            Err(reason) => Err(reason),
        }
    }
}

/// The root of the repository holding `suites`, resolved: git's top level for it, or, only
/// when git finds no repository or cannot run, the parent of its nearest `evals` ancestor.
/// The reason for no root is fixed text, naming no absolute path.
fn find_root(suites: &Path) -> Result<PathBuf, String> {
    let resolved = fs::canonicalize(suites)
        .map_err(|error| format!("the suites directory cannot be resolved: {error}"))?;
    let no_repository = match paths::worktree_root_within(&resolved, GIT_TIMEOUT) {
        Ok(top) => {
            return fs::canonicalize(top).map_err(|error| {
                format!("git's top level for the suites cannot be resolved: {error}")
            });
        }
        Err(PathError::NotARepository { .. }) => "the suites are in no git repository",
        Err(PathError::Git(_not_run)) => {
            "git could not be run to find the repository the suites are in"
        }
        Err(PathError::GitTimeout { timeout }) => {
            return Err(format!(
                "git did not name the suites' repository within {}s",
                timeout.as_secs()
            ));
        }
        Err(
            PathError::GitFailed { .. }
            | PathError::NonUtf8Output(_)
            | PathError::NoHome
            | PathError::RelativeHome { .. },
        ) => return Err("git failed to name the suites' repository".to_owned()),
    };
    resolved
        .ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "evals"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| no_repository.to_owned())
}

/// The fixture `name` under `suites`, as the absolute path a scaffold copies from, or why it
/// cannot seed a run: `<suites>/fixtures/<name>` must be a plain directory (no symlink at
/// either level) holding, within [`MAX_ENTRIES`] entries, at least one file or symlink to a
/// file or directory, every symlink resolving inside `root` and never to a directory it is
/// inside. Symlinked directories are read through, so their links are checked too.
///
/// # Errors
/// Why the fixture is refused, one line naming its path under `suites`.
pub(crate) fn locate(
    suites: &Path,
    name: &FixtureName,
    root: &RepoRoot<'_>,
) -> Result<String, String> {
    let fixtures = suites.join(FIXTURES_DIR);
    let dir = fixtures.join(name.as_str());
    let shown = format!("{FIXTURES_DIR}/{name}");
    for (path, shown) in [(&fixtures, FIXTURES_DIR), (&dir, shown.as_str())] {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(format!("no fixture directory {FIXTURES_DIR}/{name}"));
            }
            Err(error) => return Err(format!("{shown}: {error}")),
            Ok(meta) if meta.is_symlink() => {
                return Err(format!(
                    "{shown} is a symlink; a fixture is a plain directory"
                ));
            }
            Ok(meta) if !meta.is_dir() => return Err(format!("{shown} is not a directory")),
            Ok(_) => {}
        }
    }
    let walk = Walk {
        suites,
        root,
        shown: &shown,
    };
    if !walk.has_content(&dir)? {
        return Err(format!("{shown} holds no file"));
    }
    let absolute = std::path::absolute(&dir).map_err(|error| format!("{shown}: {error}"))?;
    absolute
        .into_os_string()
        .into_string()
        .map_err(|_unshown| format!("{shown}: its absolute path is not UTF-8"))
}

/// One fixture's walk.
struct Walk<'a> {
    suites: &'a Path,
    root: &'a RepoRoot<'a>,
    /// The fixture as messages name it.
    shown: &'a str,
}

/// A directory still to read, with the resolved directories it is inside.
struct Pending {
    path: PathBuf,
    inside: Vec<PathBuf>,
}

impl Walk<'_> {
    /// Whether the fixture at `dir` holds a file or a symlink to one or to a directory.
    fn has_content(&self, dir: &Path) -> Result<bool, String> {
        let resolved = fs::canonicalize(dir).map_err(|error| self.fault(dir, &error))?;
        let mut pending = vec![Pending {
            path: dir.to_path_buf(),
            inside: vec![resolved],
        }];
        let mut entries: usize = 0;
        let mut content = false;
        while let Some(next) = pending.pop() {
            let listing =
                fs::read_dir(&next.path).map_err(|error| self.fault(&next.path, &error))?;
            for entry in listing {
                let entry = entry.map_err(|error| self.fault(&next.path, &error))?;
                entries = entries.saturating_add(1);
                if entries > MAX_ENTRIES {
                    return Err(format!(
                        "{} holds more than {MAX_ENTRIES} entries",
                        self.shown
                    ));
                }
                let path = entry.path();
                let kind = entry
                    .file_type()
                    .map_err(|error| self.fault(&path, &error))?;
                let target = if kind.is_symlink() {
                    content = true;
                    self.link_target(&path, &next.inside)?
                } else if kind.is_dir() {
                    fs::canonicalize(&path).map_err(|error| self.fault(&path, &error))?
                } else if kind.is_file() {
                    content = true;
                    continue;
                } else {
                    return Err(format!(
                        "{} is not a file, directory or symlink",
                        self.relative(&path)
                    ));
                };
                if target.is_dir() {
                    // Each branch keeps its own chain: the same directory under two links is
                    // no loop.
                    let mut inside = next.inside.clone();
                    inside.push(target);
                    pending.push(Pending { path, inside });
                }
            }
        }
        Ok(content)
    }

    /// Where the symlink at `path`, read below the resolved directories `inside`, resolves.
    fn link_target(&self, path: &Path, inside: &[PathBuf]) -> Result<PathBuf, String> {
        let target = match fs::canonicalize(path) {
            Ok(target) => target,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(format!("{} is a dangling symlink", self.relative(path)));
            }
            Err(error) => return Err(self.fault(path, &error)),
        };
        let root = self.root.get().map_err(|reason| {
            format!(
                "{} is a symlink, and no repository root was found to check it against: \
                 {reason}",
                self.relative(path)
            )
        })?;
        if !target.starts_with(root) {
            return Err(format!(
                "{} leads out of the repository",
                self.relative(path)
            ));
        }
        if inside.contains(&target) {
            return Err(format!(
                "{} leads back into a directory it is inside",
                self.relative(path)
            ));
        }
        Ok(target)
    }

    /// `path` as messages name it: relative to the suites directory, with `/`.
    fn relative(&self, path: &Path) -> String {
        let relative = path.strip_prefix(self.suites).unwrap_or(path);
        crate::path_text::slashed(&relative.to_string_lossy(), std::path::MAIN_SEPARATOR)
            .into_owned()
    }

    /// The fault for an I/O `error` at `path`.
    fn fault(&self, path: &Path, error: &io::Error) -> String {
        format!("{}: {error}", self.relative(path))
    }
}

#[cfg(test)]
mod tests;
