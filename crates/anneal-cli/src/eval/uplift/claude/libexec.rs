//! The running `anneal`, placed in each plugin copy where the plugin's launcher
//! (`plugin/libexec/anneal`) looks first, so the hooks and skills of a run find a binary: the
//! launcher's release download fails inside the eval sandbox.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::scaffold::make_executable;

/// A target directory of `libexec/` that the launcher tries first on some host, and the
/// binary's file name there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LauncherTarget {
    triple: &'static str,
    file: &'static str,
}

impl LauncherTarget {
    /// The target the launcher tries first on a host whose `std::env::consts` `OS` and `ARCH`
    /// are `os` and `arch`, or `None` for a host it does not know (it lists Linux and macOS on
    /// x86-64 and ARM64, and Windows on x86-64).
    pub(crate) fn of(os: &str, arch: &str) -> Option<Self> {
        let (triple, file) = match (os, arch) {
            ("linux", "x86_64") => ("x86_64-unknown-linux-musl", "anneal"),
            ("linux", "aarch64") => ("aarch64-unknown-linux-musl", "anneal"),
            ("macos", "aarch64") => ("aarch64-apple-darwin", "anneal"),
            ("macos", "x86_64") => ("x86_64-apple-darwin", "anneal"),
            ("windows", "x86_64") => ("x86_64-pc-windows-msvc", "anneal.exe"),
            _ => return None,
        };
        Some(Self { triple, file })
    }

    /// The target of this host, if the launcher knows it.
    pub(crate) fn host() -> Option<Self> {
        Self::of(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// Its target triple, the directory under `libexec/`.
    pub(crate) const fn triple(self) -> &'static str {
        self.triple
    }

    /// Where the launcher looks, relative to a plugin's root.
    fn relative(self) -> PathBuf {
        Path::new("libexec").join(self.triple).join(self.file)
    }
}

/// The binary to place in each plugin copy, and the launcher target to place it under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement<'a> {
    /// The binary: the running `anneal` (`std::env::current_exe`).
    pub(crate) binary: &'a Path,
    /// The host's launcher target.
    pub(crate) target: LauncherTarget,
}

impl Placement<'_> {
    /// Places the binary in the plugin copy at `copy` ([`install`]).
    ///
    /// # Errors
    /// As [`install`].
    pub(crate) fn place_in(self, copy: &Path) -> io::Result<()> {
        install(self.binary, copy, self.target)
    }
}

/// Copies the binary at `binary` into the plugin copy at `copy`, where the launcher tries
/// first on `target`'s host, replacing any binary there, executable on unix.
///
/// # Errors
/// The I/O error of making the directory, removing the old binary or copying the new one.
pub(crate) fn install(binary: &Path, copy: &Path, target: LauncherTarget) -> io::Result<()> {
    let placed = copy.join(target.relative());
    if let Some(dir) = placed.parent() {
        fs::create_dir_all(dir)?;
    }
    // A read-only binary the source plugin carried cannot be opened for writing: remove it.
    if let Err(error) = fs::remove_file(&placed)
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(error);
    }
    fs::copy(binary, &placed)?;
    make_executable(&placed)
}

#[cfg(test)]
mod tests;
