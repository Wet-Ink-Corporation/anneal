//! The temporary workspace a Tier 2 run works in: a fresh directory under the system's
//! temporary directory holding a copy of the plugin, removed by [`Workspace::finish`].

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
        let workspace = Self { root: fresh_dir()? };
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

/// Makes a new, empty directory under the temporary directory; never reuses an existing one.
fn fresh_dir() -> io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let base = std::env::temp_dir();
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    for attempt in 0..ATTEMPTS {
        let path = base.join(format!(
            "anneal-eval-routing-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
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
