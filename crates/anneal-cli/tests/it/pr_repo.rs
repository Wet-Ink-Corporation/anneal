//! A scratch repository shaped like a pull request: `main` with a base commit and a `feature`
//! branch with one code change, plus helpers to rebase it (a rebase-only change), change its
//! code (a real change), and run `anneal` and its merge gate there.

use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::anneal_in;

/// A test's result.
pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The scratch repository, removed when dropped.
pub(crate) struct PrRepo {
    dir: TempDir,
}

impl PrRepo {
    /// `main` with `README.md`, then `feature` checked out with `src/lib.rs` added.
    pub(crate) fn new() -> TestResult<Self> {
        let repo = Self {
            dir: tempfile::tempdir()?,
        };
        repo.git(&["init", "-q"])?;
        repo.write("README.md", "scratch\n")?;
        repo.commit("base")?;
        repo.git(&["checkout", "-q", "-b", "feature"])?;
        repo.write("src/lib.rs", "pub fn answer() -> u32 {\n    42\n}\n")?;
        repo.commit("feature")?;
        Ok(repo)
    }

    /// The working tree's root.
    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The ledger every worktree of the repository shares.
    pub(crate) fn ledger(&self) -> PathBuf {
        self.path()
            .join(".git")
            .join("anneal")
            .join("ledger.sqlite3")
    }

    /// Runs `git <args>` isolated from the caller's repository and identity; returns stdout
    /// trimmed, or an error with git's stderr.
    pub(crate) fn git(&self, args: &[&str]) -> TestResult<String> {
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
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .current_dir(self.path())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE")
            .output()?;
        if !output.status.success() {
            return Err(
                format!("git {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into(),
            );
        }
        Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
    }

    /// Writes `content` to `rel`, creating its directories.
    pub(crate) fn write(&self, rel: &str, content: &str) -> TestResult {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(std::fs::write(path, content)?)
    }

    /// Commits everything and returns the new head.
    pub(crate) fn commit(&self, message: &str) -> TestResult<String> {
        self.git(&["add", "-A"])?;
        self.git(&["commit", "-q", "--allow-empty", "-m", message])?;
        self.head()
    }

    /// The commit `HEAD` names.
    pub(crate) fn head(&self) -> TestResult<String> {
        self.git(&["rev-parse", "HEAD"])
    }

    /// Moves `main` on with an unrelated commit and rebases `feature` onto it; returns the new
    /// head.
    pub(crate) fn rebase(&self) -> TestResult<String> {
        self.git(&["checkout", "-q", "main"])?;
        self.write("docs/other.md", "unrelated\n")?;
        self.commit("main moves on")?;
        self.git(&["checkout", "-q", "feature"])?;
        self.git(&["rebase", "-q", "main"])?;
        self.head()
    }

    /// Changes the feature's code in a new commit; returns the new head.
    pub(crate) fn change_code(&self) -> TestResult<String> {
        self.write("src/lib.rs", "pub fn answer() -> u32 {\n    43\n}\n")?;
        self.commit("real change")
    }

    /// Runs `anneal <args>` here.
    pub(crate) fn anneal(&self, args: &[&str]) -> TestResult<Output> {
        Ok(anneal_in(self.path()).args(args).output()?)
    }

    /// Runs `anneal <args>`, expecting exit `code`, nothing on stderr and an envelope.
    pub(crate) fn envelope(&self, args: &[&str], code: i32) -> TestResult<Value> {
        let output = self.anneal(args)?;
        assert_eq!(
            output.status.code(),
            Some(code),
            "anneal {args:?}: {output:?}"
        );
        assert!(output.stderr.is_empty(), "anneal {args:?}: {output:?}");
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    /// `anneal verdict set` for pull request 7 at `HEAD` (exit 0).
    pub(crate) fn set_verdict(&self, level: &str, source: &str) -> TestResult<Value> {
        self.envelope(
            &[
                "verdict", "set", "--pr", "7", "--sha", "HEAD", "--level", level, "--source",
                source,
            ],
            0,
        )
    }

    /// Runs `anneal hook merge-gate --fail-closed` on a Bash call of `command`.
    pub(crate) fn merge_gate(&self, command: &str) -> TestResult<Output> {
        let input = serde_json::to_vec(&json!({
            "session_id": "s-it",
            "transcript_path": "/dev/null",
            "cwd": self.path(),
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "tool_use_id": "toolu_1",
        }))?;
        self.hook_stdin(&input)
    }

    /// Runs `anneal hook merge-gate --fail-closed` with `stdin`.
    pub(crate) fn hook_stdin(&self, stdin: &[u8]) -> TestResult<Output> {
        let mut child = anneal_in(self.path())
            .args(["hook", "merge-gate", "--fail-closed"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().ok_or("stdin is piped")?;
        input.write_all(stdin)?;
        drop(input);
        Ok(child.wait_with_output()?)
    }
}
