//! Shared helpers: scratch git repositories, fixture copies, content hashes and an isolated
//! `anneal` command.

use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

/// Runs `git <args>` in `dir`, isolated from the caller's repository and identity, and panics
/// with git's stderr if it fails.
pub(crate) fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=anneal test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `anneal` run in `cwd`, isolated from the caller's repository, ledger overrides, Clef
/// configuration and anneal home: the plugin's `CLAUDE_PLUGIN_OPTION_*` Clef variables are
/// removed, and `ANNEAL_HOME` is `.anneal-home` under `cwd` when `cwd` is a temporary
/// directory (else under the system's temporary directory), so no test writes to `~/.anneal`.
/// A test that needs either sets it again.
pub(crate) fn anneal_in(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_anneal"));
    let temp = std::env::temp_dir();
    let home = if cwd.starts_with(&temp) {
        cwd.join(".anneal-home")
    } else {
        temp.join(format!("anneal-it-home-{}", std::process::id()))
    };
    command
        .current_dir(cwd)
        .env("ANNEAL_HOME", home)
        .env_remove("ANNEAL_STORE")
        .env_remove("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID")
        .env_remove("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN")
        .env_remove("CLAUDE_PLUGIN_OPTION_CLEF_MODE")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE");
    command
}

/// A repository at `<root>/main` with one commit, and a linked worktree of it at
/// `<root>/linked`.
pub(crate) fn repo_with_linked_worktree(root: &Path) {
    let main = root.join("main");
    std::fs::create_dir(&main).expect("create main");
    git(&main, &["init", "-q"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "init"]);
    git(
        &main,
        &["worktree", "add", "-q", "-b", "linked", "../linked"],
    );
}

/// `sha256:` and the lowercase hex digest of `bytes`, as the ledger records content hashes.
pub(crate) fn sha256_hex(bytes: &[u8]) -> Result<String, std::fmt::Error> {
    let mut hex = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}")?;
    }
    Ok(hex)
}

/// Copies the directory tree at `from` to `to`, creating `to`.
pub(crate) fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
