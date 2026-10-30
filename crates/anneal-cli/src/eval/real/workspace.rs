//! The temporary workspace a Tier 2 or Tier 3 run works in: a fresh directory under the
//! system's temporary directory (Tier 2) or the run's work directory (Tier 3) holding a copy
//! of the plugin, removed by [`Workspace::finish`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many names are tried for a fresh directory before giving up.
const ATTEMPTS: u32 = 16;

/// The most a copied tree may hold (H-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Files and directories.
    pub(crate) entries: u64,
    /// Bytes of file content.
    pub(crate) bytes: u64,
}

impl Limits {
    /// A plugin: 10,000 entries and 512 MiB, room for built binaries in `libexec/`.
    pub(crate) const PLUGIN: Self = Self {
        entries: 10_000,
        bytes: 512 * 1024 * 1024,
    };
}

/// A fresh temporary directory holding a copy of the plugin under test.
///
/// It has no `Drop`: the run calls [`Workspace::finish`] on every path, so a failure to remove
/// it is reported rather than lost.
#[derive(Debug)]
pub(crate) struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// A fresh directory under the system's temporary directory, holding a copy of `plugin`
    /// at [`Workspace::plugin`].
    ///
    /// # Errors
    /// The I/O error that stopped the directory being made or the plugin copied (a symlink in
    /// the plugin is `InvalidInput`; a plugin over [`Limits::PLUGIN`] is `FileTooLarge`). The
    /// directory is removed again when the copy fails.
    pub(crate) fn create(plugin: &Path) -> io::Result<Self> {
        Self::create_in(&std::env::temp_dir(), "routing", plugin)
    }

    /// [`Workspace::create`] under `base` (made first if missing) for the suite named
    /// `suite`, whose directory is `anneal-eval-<suite>-*`.
    ///
    /// # Errors
    /// As [`Workspace::create`], and the I/O error that stopped `base` being made.
    pub(crate) fn create_in(base: &Path, suite: &str, plugin: &Path) -> io::Result<Self> {
        fs::create_dir_all(base)?;
        let workspace = Self {
            root: fresh_dir(base, suite)?,
        };
        match copy_tree(plugin, &workspace.plugin(), Limits::PLUGIN) {
            Ok(()) => Ok(workspace),
            Err(error) => Err(match workspace.finish() {
                Ok(()) => error,
                Err(removing) => io::Error::new(
                    error.kind(),
                    format!("{error}; removing the workspace also failed: {removing}"),
                ),
            }),
        }
    }

    /// The workspace directory.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The plugin's copy.
    pub(crate) fn plugin(&self) -> PathBuf {
        self.root.join("plugin")
    }

    /// Removes the workspace and everything in it.
    ///
    /// # Errors
    /// The I/O error that stopped the removal.
    pub(crate) fn finish(self) -> io::Result<()> {
        fs::remove_dir_all(&self.root)
    }
}

/// Makes a new, empty directory under `base`; never reuses an existing one.
fn fresh_dir(base: &Path, suite: &str) -> io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    for attempt in 0..ATTEMPTS {
        let path = base.join(format!(
            "anneal-eval-{suite}-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match create_private_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

/// Makes the directory at `path`, on unix readable, writable and searchable by its owner
/// only (0700), so no other local account can plant files or links in a workspace that sits
/// in the shared temporary directory.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new().mode(0o700).create(path)
}

/// Makes the directory at `path`; elsewhere the temporary directory is already per user.
#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

/// Copies the directory `from` to the new directory `to`, file by file, within `limits`.
///
/// # Errors
/// The I/O error of a read or write; `InvalidInput` for a symlink or anything that is neither
/// a file nor a directory; `FileTooLarge` past `limits`.
pub(crate) fn copy_tree(from: &Path, to: &Path, limits: Limits) -> io::Result<()> {
    let mut entries: u64 = 0;
    let mut bytes: u64 = 0;
    let mut pending = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, target)) = pending.pop() {
        fs::create_dir(&target)?;
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            entries = entries.saturating_add(1);
            if entries > limits.entries {
                return Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    format!("the plugin holds more than {} entries", limits.entries),
                ));
            }
            let kind = entry.file_type()?;
            let (path, copy) = (entry.path(), target.join(entry.file_name()));
            if kind.is_dir() {
                pending.push((path, copy));
            } else if kind.is_file() {
                bytes = bytes.saturating_add(entry.metadata()?.len());
                if bytes > limits.bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::FileTooLarge,
                        format!("the plugin holds more than {} bytes", limits.bytes),
                    ));
                }
                // The source is the user's own repository, so a link swapped in after the
                // type check, or a file growing past the count, is accepted as low risk.
                fs::copy(&path, &copy)?;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} is a symlink or special file; the plugin copy takes plain files only",
                        path.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use super::{Limits, Workspace, copy_tree};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    #[cfg(unix)]
    fn the_workspace_is_private_to_its_owner() -> TestResult {
        use std::os::unix::fs::PermissionsExt;

        let source = tempfile::tempdir()?;

        let workspace = Workspace::create(source.path())?;

        let mode = fs::metadata(workspace.root())?.permissions().mode() & 0o777;
        workspace.finish()?;
        assert_eq!(mode, 0o700, "{mode:o}");
        Ok(())
    }

    #[test]
    fn the_plugin_is_copied_into_a_fresh_directory_that_finish_removes() -> TestResult {
        let source = tempfile::tempdir()?;
        fs::create_dir_all(source.path().join("skills/plan"))?;
        fs::write(source.path().join("skills/plan/SKILL.md"), "plan")?;
        fs::write(source.path().join("README.md"), "readme")?;

        let workspace = Workspace::create(source.path())?;

        let copy = workspace.plugin();
        assert_eq!(
            (
                fs::read_to_string(copy.join("skills/plan/SKILL.md"))?,
                fs::read_to_string(copy.join("README.md"))?
            ),
            ("plan".to_owned(), "readme".to_owned())
        );
        let root = workspace.root().to_path_buf();
        workspace.finish()?;
        assert!(!root.exists(), "removed");
        Ok(())
    }

    #[test]
    fn a_workspace_made_in_a_base_that_does_not_exist_yet_makes_the_base() -> TestResult {
        let source = tempfile::tempdir()?;
        let parent = tempfile::tempdir()?;
        let base = parent.path().join("cache/tmp");

        let workspace = Workspace::create_in(&base, "uplift", source.path())?;

        let (inside, name) = (
            workspace.root().parent() == Some(base.as_path()),
            workspace
                .root()
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("anneal-eval-uplift-")),
        );
        workspace.finish()?;
        assert_eq!((inside, name, base.is_dir()), (true, true, true));
        Ok(())
    }

    #[test]
    fn a_tree_over_the_entry_or_byte_limit_is_refused() -> TestResult {
        let source = tempfile::tempdir()?;
        fs::write(source.path().join("a"), "12345")?;
        fs::write(source.path().join("b"), "12345")?;
        let target = tempfile::tempdir()?;

        let entries = copy_tree(
            source.path(),
            &target.path().join("one"),
            Limits {
                entries: 1,
                bytes: 100,
            },
        );
        let bytes = copy_tree(
            source.path(),
            &target.path().join("two"),
            Limits {
                entries: 10,
                bytes: 9,
            },
        );

        assert_eq!(
            (entries.map_err(|e| e.kind()), bytes.map_err(|e| e.kind())),
            (
                Err(io::ErrorKind::FileTooLarge),
                Err(io::ErrorKind::FileTooLarge)
            )
        );
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_in_the_plugin_is_refused() -> TestResult {
        let source = tempfile::tempdir()?;
        std::os::unix::fs::symlink("/etc", source.path().join("link"))?;
        let target = tempfile::tempdir()?;

        let copied = copy_tree(source.path(), &target.path().join("copy"), Limits::PLUGIN);

        assert_eq!(
            copied.map_err(|e| e.kind()),
            Err(io::ErrorKind::InvalidInput)
        );
        Ok(())
    }
}
