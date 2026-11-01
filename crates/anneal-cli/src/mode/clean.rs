//! Whether a worktree is clean when build mode is entered.
//!
//! Clean means `git status` reports no change, untracked (not ignored) files included, except
//! untracked character devices. git cannot commit a device, so it is never a change; Claude
//! Code's Bash sandbox mounts `/dev/null` over the dotfiles it protects in the working
//! directory (`.bashrc`, `.gitconfig`, `.claude/settings.json`, ...), and without this every
//! sandboxed session would enter build mode dirty.

use std::path::Path;

use crate::paths::{self, GIT_TIMEOUT, PathError};

/// Most bytes of `git status --porcelain -z` read. A longer answer is taken as dirty without
/// looking further: the sandbox's devices fit well inside it.
const STATUS_CAP_BYTES: u64 = 4096;

/// Whether the worktree at `root` has no change git reports, apart from untracked character
/// devices.
///
/// `--untracked-files=all` lists each untracked file, so a device inside an untracked
/// directory (the sandbox's `.claude/settings.json`) can be told apart from a real file. It
/// also makes git walk every untracked directory that is not ignored: a very large one can
/// reach [`GIT_TIMEOUT`], and then `anneal mode build` fails with that error rather than
/// recording the tree, which still fails closed.
///
/// # Errors
/// [`PathError`] when git cannot be run, times out or fails.
pub(crate) fn is_clean(root: &Path) -> Result<bool, PathError> {
    let output = paths::run_git_capped(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        GIT_TIMEOUT,
        STATUS_CAP_BYTES.saturating_add(1),
    )?;
    if !output.status.success() {
        return Err(paths::git_failure(root, &output.stderr));
    }
    Ok(porcelain_is_clean(
        &output.stdout,
        STATUS_CAP_BYTES,
        |path| is_device(root, path),
    ))
}

/// Whether `stdout` of `git status --porcelain=v1 -z` (at most `cap` bytes) lists only
/// untracked paths for which `is_device` holds. Any other entry, including a rename's second
/// path, is a change.
fn porcelain_is_clean(stdout: &[u8], cap: u64, is_device: impl Fn(&[u8]) -> bool) -> bool {
    if u64::try_from(stdout.len()).map_or(true, |len| len > cap) {
        return false;
    }
    stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .all(|entry| entry.strip_prefix(b"?? ").is_some_and(&is_device))
}

/// Whether `path` (relative to `root`, as git prints it) is itself a character device, not a
/// link to one. A path whose metadata cannot be read counts as not a device, so the tree is
/// dirty: failing closed is the answer the hook needs when it cannot tell.
#[cfg(unix)]
fn is_device(root: &Path, path: &[u8]) -> bool {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::FileTypeExt as _;

    std::fs::symlink_metadata(root.join(OsStr::from_bytes(path)))
        .is_ok_and(|metadata| metadata.file_type().is_char_device())
}

/// Windows has no sandbox devices in the worktree: nothing untracked is exempt.
#[cfg(not(unix))]
fn is_device(_root: &Path, _path: &[u8]) -> bool {
    false
}

#[cfg(test)]
mod tests;
