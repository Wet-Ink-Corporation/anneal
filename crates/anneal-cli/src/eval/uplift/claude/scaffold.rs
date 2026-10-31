//! The scaffold script a fixture case's runs start with: `claude plugin eval` runs it (only with
//! `--scaffold`) in the run's empty workspace, outside the eval sandbox and with no `EVAL_*`
//! variables, so it names the fixture by its absolute path.

use std::fs;
use std::io;
use std::path::Path;

/// The script's file name, beside the case's `case.yaml`.
pub(crate) const SCRIPT_FILE: &str = "scaffold.sh";

/// The script that copies the fixture at the absolute path `fixture` into the current
/// directory, symlinks dereferenced, and commits it as a fresh git repository. LF only.
pub(crate) fn script(fixture: &str) -> String {
    let source = single_quoted(&format!("{fixture}/."));
    format!(
        "#!/bin/sh\n\
         # Written by anneal eval uplift: seeds the run's empty workspace with the case's fixture.\n\
         set -eu\n\
         cp -RL {source} .\n\
         git init -q -b main\n\
         git add -A\n\
         git -c user.name=eval -c user.email=eval@invalid -c commit.gpgsign=false \
         commit -qm fixture\n"
    )
}

/// `text` as one single-quoted shell word: each `'` closes the quote, is escaped, and reopens it.
fn single_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Writes the script for the fixture at `fixture` into `case_dir`, executable on unix.
///
/// # Errors
/// The I/O error of the write or of setting its mode.
pub(crate) fn write(case_dir: &Path, fixture: &str) -> io::Result<()> {
    let path = case_dir.join(SCRIPT_FILE);
    fs::write(&path, script(fixture))?;
    make_executable(&path)
}

/// Makes the file at `path` readable and executable by everyone, writable by its owner.
#[cfg(unix)]
pub(crate) fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

/// Elsewhere a file's mode does not decide whether it runs, so the file need only exist.
#[cfg(not(unix))]
pub(crate) fn make_executable(path: &Path) -> io::Result<()> {
    fs::metadata(path).map(drop)
}

#[cfg(test)]
mod tests;
