//! Running `claude plugin eval` in a temporary workspace, as both tiers do.
//!
//! The plugin is copied into a fresh temporary workspace, since `claude plugin eval` reads its
//! cases from a directory below the plugin and writes its results there; the repository's
//! `plugin/` stays untouched. Each run is [`invoke`]d with stdin closed, stdout discarded and
//! stderr into a file (nothing is piped, so a grandchild that outlives a killed child blocks
//! nothing), within what is left of the time limit. The result is read from the file `--json`
//! names.
//!
//! At the time limit only `claude` itself is killed and reaped. Sessions it started as child
//! processes, if any, keep running until their own 180 s timeout and may still spend, so a
//! killed batch's cost is unknown and the run's `cost_usd` is reported as a lower bound.
//! Killing the whole process group would need `kill(2)` on a negative pid, which std does
//! not offer and this crate has no dependency for (H-01, H-16); putting `claude` in its own
//! group without that would only stop a terminal's Ctrl-C reaching it.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::RouterError;
use crate::json_file;
use crate::process::{self, WaitError};

/// The most of a result file read (H-13).
const MAX_RESULT_BYTES: u64 = 64 * 1024 * 1024;
/// The most of a plugin manifest read.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// The most of the child's stderr kept for an error message.
const STDERR_TAIL_BYTES: u64 = 2048;
/// The most characters of that tail shown.
const STDERR_TAIL_CHARS: usize = 300;

/// How `claude plugin eval` is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Options<'a> {
    /// The `claude` program.
    pub(crate) program: &'a Path,
    /// The model every run uses, or `None` for Claude Code's default.
    pub(crate) model: Option<&'a str>,
    /// Runs at once (1 to 8).
    pub(crate) concurrency: u8,
    /// Pass `--trust-plugin`: the operator vouches for the plugin and its suite.
    pub(crate) trust_plugin: bool,
    /// The time every batch together may take.
    pub(crate) time_limit: Duration,
}

/// The plugin manifest, the field read.
#[derive(Debug, Deserialize)]
struct ManifestDto {
    name: String,
}

/// The name in `<plugin>/.claude-plugin/plugin.json`.
///
/// # Errors
/// [`RouterError::Manifest`] when it cannot be read or has no string `name`.
pub(crate) fn plugin_name(plugin: &Path) -> Result<String, RouterError> {
    let manifest: ManifestDto = json_file::read(
        &plugin.join(".claude-plugin").join("plugin.json"),
        MAX_MANIFEST_BYTES,
    )
    .map_err(RouterError::Manifest)?;
    Ok(manifest.name)
}

/// One run of `claude` in a workspace.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Invocation<'a> {
    /// The `claude` program.
    pub(crate) program: &'a Path,
    /// Its arguments, `--json` naming [`Invocation::result_path`].
    pub(crate) args: &'a [OsString],
    /// The workspace it runs in.
    pub(crate) root: &'a Path,
    /// Names this run's files in the workspace: `result-<tag>.json`, `stderr-<tag>.log`.
    pub(crate) tag: &'a str,
}

/// Where `--json` writes the result of the run tagged `tag` in the workspace at `root`: the
/// one place both the argument and the reader take it from.
pub(crate) fn result_path(root: &Path, tag: &str) -> std::path::PathBuf {
    root.join(format!("result-{tag}.json"))
}

impl Invocation<'_> {
    /// Where `--json` writes the result ([`result_path`]).
    pub(crate) fn result_path(&self) -> std::path::PathBuf {
        result_path(self.root, self.tag)
    }

    fn stderr_path(&self) -> std::path::PathBuf {
        self.root.join(format!("stderr-{}.log", self.tag))
    }
}

/// What a `claude plugin eval` run came back with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Invoked {
    /// Its result file, parsed as JSON.
    Result(Value),
    /// It was killed at the time limit, before it reported anything.
    TimedOut,
}

/// Runs `invocation` with stdin closed, stdout discarded and stderr into a file (nothing is
/// piped, so a grandchild that outlives a killed child blocks nothing), within `left`, and
/// reads the result file `--json` named.
///
/// # Errors
/// [`RouterError::Spawn`] when `claude` cannot start, [`RouterError::Wait`] when it cannot be
/// waited on, [`RouterError::Failed`] when it wrote no result, [`RouterError::Unreadable`]
/// when the result is not JSON within the cap, and [`RouterError::Workspace`] when the stderr
/// log cannot be created as a new file.
pub(crate) fn invoke(invocation: &Invocation<'_>, left: Duration) -> Result<Invoked, RouterError> {
    let stderr_path = invocation.stderr_path();
    // A fresh file only: never one planted in the workspace, nor a link it follows.
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stderr_path)
        .map_err(RouterError::Workspace)?;
    let mut child = Command::new(invocation.program)
        .args(invocation.args)
        .current_dir(invocation.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|source| RouterError::Spawn {
            // The error outlives the borrowed arguments, so it owns the path.
            program: invocation.program.to_path_buf(),
            source,
        })?;
    // Nothing is piped, so the wait returns once the child itself is reaped.
    let status = match process::wait_with_deadline(&mut child, left, 0, "claude") {
        Ok(output) => output.status,
        Err(WaitError::Timeout) => return Ok(Invoked::TimedOut),
        Err(WaitError::Io(error)) => return Err(RouterError::Wait(error)),
    };
    let result_path = invocation.result_path();
    if !result_path.is_file() {
        return Err(RouterError::Failed {
            status: status.to_string(),
            stderr: stderr_tail(&stderr_path),
        });
    }
    json_file::read(&result_path, MAX_RESULT_BYTES)
        .map(Invoked::Result)
        .map_err(RouterError::Unreadable)
}

/// `budget_usd` rounded down to the cent, so a batch never gets more than is left.
pub(crate) fn budget_arg(budget_usd: f64) -> String {
    format!("{:.2}", (budget_usd * 100.0).floor() / 100.0)
}

/// [`tail`] of the log at `path`, or a placeholder naming why it could not be read, so a
/// failed `claude` is still reported as such.
fn stderr_tail(path: &Path) -> String {
    tail(path).unwrap_or_else(|error| format!("(stderr unreadable: {error})"))
}

/// The last non-blank line of the file at `path`, within its last [`STDERR_TAIL_BYTES`],
/// control characters dropped and cut to [`STDERR_TAIL_CHARS`]; `(no stderr)` when empty.
fn tail(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::Start(len.saturating_sub(STDERR_TAIL_BYTES)))?;
    let mut bytes = Vec::new();
    file.take(STDERR_TAIL_BYTES).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("(no stderr)");
    Ok(line
        .chars()
        .filter(|c| !c.is_control())
        .take(STDERR_TAIL_CHARS)
        .collect())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{budget_arg, stderr_tail};

    #[test]
    fn the_budget_is_rounded_down_to_the_cent() {
        assert_eq!(
            [budget_arg(5.0), budget_arg(0.019), budget_arg(1.999)],
            ["5.00", "0.01", "1.99"]
        );
    }

    #[test]
    fn an_unreadable_stderr_log_is_named_in_the_failure_rather_than_replacing_it() {
        let tail = stderr_tail(Path::new("no/such/stderr.log"));

        assert!(tail.starts_with("(stderr unreadable: "), "{tail}");
    }
}
