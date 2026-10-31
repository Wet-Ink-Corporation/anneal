//! The temporary workspace a Tier 2 or Tier 3 run works in: a fresh directory under the
//! system's temporary directory (Tier 2) or the run's work directory (Tier 3) holding a copy
//! of the plugin, removed by [`Workspace::finish`].

use std::fs;
use std::hash::{BuildHasher, RandomState};
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
    /// A fresh directory under the system's temporary directory, named
    /// [`ROUTING_PREFIX`] and hex digits, holding a copy of `plugin` at [`Workspace::plugin`].
    ///
    /// # Errors
    /// The I/O error that stopped the directory being made or the plugin copied (a symlink in
    /// the plugin is `InvalidInput`; a plugin over [`Limits::PLUGIN`] is `FileTooLarge`). The
    /// directory is removed again when the copy fails.
    pub(crate) fn create(plugin: &Path) -> io::Result<Self> {
        Self::create_in(&std::env::temp_dir(), ROUTING_PREFIX, plugin)
    }

    /// [`Workspace::create`] under `base` (made first if missing), the directory named
    /// `prefix` and hex digits ([`name_len`]): each suite chooses its prefix.
    ///
    /// # Errors
    /// As [`Workspace::create`], and the I/O error that stopped `base` being made.
    pub(crate) fn create_in(base: &Path, prefix: &str, plugin: &Path) -> io::Result<Self> {
        fs::create_dir_all(base)?;
        let workspace = Self {
            root: fresh_dir(base, prefix)?,
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

    /// Removes the workspace and everything in it. A directory it may not write is opened to
    /// its owner first (`claude plugin eval --keep-temp` seals each kept run that way).
    ///
    /// # Errors
    /// The I/O error that stopped the removal.
    pub(crate) fn finish(self) -> io::Result<()> {
        remove(&self.root)
    }
}

/// Removes `root`, opening any directory it may not write to its owner and trying again.
#[cfg(unix)]
fn remove(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            open_up(root)?;
            fs::remove_dir_all(root)
        }
        removed => removed,
    }
}

/// Elsewhere directory modes do not stop a removal, so there is nothing to open first.
#[cfg(not(unix))]
fn remove(root: &Path) -> io::Result<()> {
    fs::remove_dir_all(root)
}

/// Makes `root` and every directory below it readable, writable and searchable by its owner
/// (0700), never following a link, so the tree can be removed.
#[cfg(unix)]
fn open_up(root: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        // Checked again right before the change, not following a link: anything that is not
        // a real directory now (a link swapped in since it was listed, or a linked root) is
        // left alone. `set_permissions` follows links, so this narrows the race rather than
        // closing it; the workspace's 0700 mode limits who could swap a link in.
        if !fs::symlink_metadata(&dir)?.is_dir() {
            continue;
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            // The entry's own type: a link to a directory is not followed.
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

/// The hex digits that end a workspace's name. Short, because Tier 3's `claude` children get a
/// `TMPDIR` inside the workspace and their sandbox's socket paths must fit in 107 bytes
/// (`uplift::sandbox`); uniqueness comes from the exclusive create and its retries.
const SUFFIX_DIGITS: usize = 6;
/// The bits of the hash [`SUFFIX_DIGITS`] hex digits show.
const SUFFIX_MASK: u64 = (1 << (4 * SUFFIX_DIGITS)) - 1;

/// The prefix of a Tier 2 (`eval routing --router claude`) workspace's name. Tier 2's
/// children get no `TMPDIR` inside it, so its name has no length budget.
const ROUTING_PREFIX: &str = "anneal-eval-routing-";

/// The length in bytes of the name of a workspace whose name starts with `prefix`: the
/// prefix and [`SUFFIX_DIGITS`] hex digits.
pub(crate) const fn name_len(prefix: &str) -> usize {
    prefix.len() + SUFFIX_DIGITS
}

/// Makes a new, empty directory under `base`, named `<prefix><hex>` ([`name_len`]); never
/// reuses an existing one. The digits hash the process id, the clock and the attempt with a
/// per-process random key, so another process cannot predict them.
fn fresh_dir(base: &Path, prefix: &str) -> io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let keys = RandomState::new();
    fresh_dir_from(base, prefix, |attempt| {
        keys.hash_one((std::process::id(), nanos, attempt))
    })
}

/// [`fresh_dir`] with the digits of each attempt (from 0) taken from `suffix`, masked to
/// [`SUFFIX_DIGITS`]: a name already taken is retried with the next attempt's, up to
/// [`ATTEMPTS`], then the last `AlreadyExists` is returned.
fn fresh_dir_from(
    base: &Path,
    prefix: &str,
    mut suffix: impl FnMut(u32) -> u64,
) -> io::Result<PathBuf> {
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    for attempt in 0..ATTEMPTS {
        let digits = suffix(attempt) & SUFFIX_MASK;
        let path = base.join(format!("{prefix}{digits:0SUFFIX_DIGITS$x}"));
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

    use super::{ATTEMPTS, Limits, Workspace, copy_tree, fresh_dir_from, name_len};

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

        let workspace = Workspace::create_in(&base, "ae-", source.path())?;

        let (inside, name) = (
            workspace.root().parent() == Some(base.as_path()),
            workspace
                .root()
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ae-")),
        );
        workspace.finish()?;
        assert_eq!((inside, name, base.is_dir()), (true, true, true));
        Ok(())
    }

    #[test]
    fn a_workspace_name_is_its_suite_prefix_and_six_hex_digits() -> TestResult {
        let source = tempfile::tempdir()?;
        let base = tempfile::tempdir()?;

        let workspace = Workspace::create_in(base.path(), "anneal-eval-uplift-", source.path())?;

        let name = workspace
            .root()
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        workspace.finish()?;
        let name = name.ok_or("a UTF-8 name")?;
        let suffix = name.strip_prefix("anneal-eval-uplift-").unwrap_or_default();
        assert_eq!(
            (
                name.len(),
                name_len("anneal-eval-uplift-"),
                suffix.len(),
                suffix.chars().all(|c| c.is_ascii_hexdigit())
            ),
            (25, 25, 6, true),
            "{name}"
        );
        Ok(())
    }

    #[test]
    fn a_tier_2_workspace_keeps_its_routing_name() -> TestResult {
        let source = tempfile::tempdir()?;

        let workspace = Workspace::create(source.path())?;

        let name = workspace
            .root()
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        workspace.finish()?;
        let name = name.ok_or("a UTF-8 name")?;
        assert_eq!(
            (
                name.starts_with("anneal-eval-routing-"),
                name.len(),
                name_len("anneal-eval-routing-")
            ),
            (true, 26, 26),
            "{name}"
        );
        Ok(())
    }

    #[test]
    fn a_name_already_taken_is_retried_with_the_next_suffix() -> TestResult {
        let base = tempfile::tempdir()?;
        fs::create_dir(base.path().join("ae-000000"))?;

        let made = fresh_dir_from(base.path(), "ae-", u64::from)?;

        assert_eq!((made.is_dir(), made), (true, base.path().join("ae-000001")));
        Ok(())
    }

    #[test]
    fn every_attempt_taken_is_already_exists() -> TestResult {
        let base = tempfile::tempdir()?;
        fs::create_dir(base.path().join("ae-000000"))?;
        let mut tried: u32 = 0;

        let made = fresh_dir_from(base.path(), "ae-", |_| {
            tried += 1;
            0
        });

        assert_eq!(
            (
                made.map_err(|e| e.kind()),
                tried,
                fs::read_dir(base.path())?.count()
            ),
            (Err(io::ErrorKind::AlreadyExists), ATTEMPTS, 1)
        );
        Ok(())
    }

    #[test]
    fn two_workspaces_in_one_base_get_different_names() -> TestResult {
        let source = tempfile::tempdir()?;
        let base = tempfile::tempdir()?;

        let first = Workspace::create_in(base.path(), "ae-", source.path())?;
        let second = Workspace::create_in(base.path(), "ae-", source.path())?;

        let distinct = first.root() != second.root();
        first.finish()?;
        second.finish()?;
        assert!(distinct, "each workspace is a fresh directory");
        Ok(())
    }

    /// As `claude plugin eval --keep-temp` seals a kept run for a user other than root: the
    /// run's root read-only (0500), holding a search-only (0100) directory.
    #[test]
    #[cfg(unix)]
    fn finish_removes_directories_a_kept_run_sealed() -> TestResult {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let source = tempfile::tempdir()?;
        let workspace = Workspace::create(source.path())?;
        let run = workspace.root().join("tmp/claude-eval-abc123");
        fs::create_dir_all(run.join("sealed/home"))?;
        fs::write(run.join("sealed/home/notes"), "kept")?;
        fs::write(run.join("trace.jsonl"), "{}")?;
        fs::set_permissions(run.join("sealed"), fs::Permissions::from_mode(0o100))?;
        fs::set_permissions(&run, fs::Permissions::from_mode(0o500))?;
        let root = workspace.root().to_path_buf();
        if fs::read_dir(run.join("sealed")).is_ok() {
            // The seal does not bind this user (root ignores directory modes), so nothing
            // here would need opening up and the test would pass whatever `finish` does:
            // clean up, check the removal and say so. Run it as another user
            // (`setpriv --reuid 65534`) to check the seal itself.
            workspace.finish()?;
            writeln!(
                io::stderr().lock(),
                "skipped: the seal does not bind this user"
            )?;
            assert!(!root.exists());
            return Ok(());
        }

        let finished = workspace.finish();

        assert_eq!(
            (finished.map_err(|e| e.kind()), root.exists()),
            (Ok(()), false)
        );
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn opening_up_never_changes_what_a_link_points_at() -> TestResult {
        use std::os::unix::fs::PermissionsExt;

        let target = tempfile::tempdir()?;
        fs::set_permissions(target.path(), fs::Permissions::from_mode(0o750))?;
        let parent = tempfile::tempdir()?;
        let link = parent.path().join("link");
        std::os::unix::fs::symlink(target.path(), &link)?;

        let opened = super::open_up(&link);

        let mode = fs::metadata(target.path())?.permissions().mode() & 0o777;
        assert_eq!(
            (opened.map_err(|e| e.kind()), mode),
            (Ok(()), 0o750),
            "{mode:o}"
        );
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
