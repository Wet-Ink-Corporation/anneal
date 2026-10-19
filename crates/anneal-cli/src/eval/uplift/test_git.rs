//! Test-only: whether git can run, for the tests that need it. Without git a test is skipped
//! with a note on stderr, except under CI (`CI` set), where a missing git fails it, so a skip is
//! never silent where it matters.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;

/// `Ok(true)` when git runs; `Ok(false)`, noted on stderr, when it does not and `CI` is unset.
///
/// # Errors
/// When git does not run and `CI` is set, or stderr cannot be written.
pub(crate) fn available() -> Result<bool, Box<dyn std::error::Error>> {
    if Command::new("git").arg("--version").output().is_ok() {
        return Ok(true);
    }
    if std::env::var_os("CI").is_some() {
        return Err("git is not installed, and CI (`CI` is set) must run this test".into());
    }
    writeln!(
        std::io::stderr().lock(),
        "skipped: git is not installed (set CI to make this a failure)"
    )?;
    Ok(false)
}

/// Runs `git init -q` in `dir`, with git's global and system settings out of the way.
///
/// # Errors
/// When git cannot run or fails.
pub(crate) fn init(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", dir.join(".no-global-gitconfig"))
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("git init failed: {output:?}").into())
    }
}
