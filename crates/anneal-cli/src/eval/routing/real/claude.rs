//! The [`Router`] adapter over `claude plugin eval`.
//!
//! The plugin is copied into a fresh temporary workspace, since `claude plugin eval` reads its
//! cases from a directory below the plugin and writes its results there; the repository's
//! `plugin/` stays untouched. Each batch writes its cases to `<copy>/anneal-routing-<round>/`
//! and runs the CLI with stdin closed, stdout discarded and stderr into a file (nothing is
//! piped, so a grandchild that outlives a killed child blocks nothing), within what is left of
//! the time limit. The result is read from the file `--json` names.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::drive::{Batch, Router};
use super::result::{self, ResultDto};
use super::suite::{Graders, case};
use super::workspace::Workspace;
use super::{Ran, RouterError, Stop};
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

/// The [`Router`] that runs `claude plugin eval` in a temporary workspace.
#[derive(Debug)]
pub(crate) struct ClaudeRouter<'a> {
    options: Options<'a>,
    workspace: Workspace,
    plugin: &'a str,
    graders: &'a Graders<'a>,
    started: Instant,
}

impl<'a> ClaudeRouter<'a> {
    /// A router over a fresh copy of the plugin at `plugin_dir`, named `plugin`, whose runs
    /// are judged by `graders`. The time limit starts now.
    ///
    /// # Errors
    /// [`RouterError::Workspace`] when the workspace cannot be made or the plugin copied.
    pub(crate) fn new(
        options: Options<'a>,
        plugin_dir: &Path,
        plugin: &'a str,
        graders: &'a Graders<'a>,
    ) -> Result<Self, RouterError> {
        Ok(Self {
            options,
            workspace: Workspace::create(plugin_dir).map_err(RouterError::Workspace)?,
            plugin,
            graders,
            started: Instant::now(),
        })
    }

    /// Removes the workspace.
    ///
    /// # Errors
    /// [`RouterError::Workspace`] when it cannot be removed.
    pub(crate) fn finish(self) -> Result<(), RouterError> {
        self.workspace.finish().map_err(RouterError::Workspace)
    }

    /// Writes `batch`'s cases to `<copy>/<eval dir>/<case>/case.yaml`.
    fn write_cases(&self, batch: &Batch<'_>) -> Result<(), RouterError> {
        let dir = self.workspace.plugin().join(eval_dir(batch.round));
        for entry in &batch.cases {
            let case_dir = dir.join(&entry.name);
            fs::create_dir_all(&case_dir).map_err(RouterError::Workspace)?;
            let text = serde_json::to_vec_pretty(&case(
                &entry.name,
                entry.prompt,
                batch.runs,
                self.graders,
            ))
            .map_err(|error| RouterError::Workspace(io::Error::other(error)))?;
            fs::write(case_dir.join("case.yaml"), text).map_err(RouterError::Workspace)?;
        }
        Ok(())
    }
}

impl Router for ClaudeRouter<'_> {
    fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError> {
        let left = self
            .options
            .time_limit
            .saturating_sub(self.started.elapsed());
        if left.is_zero() {
            return Ok(stopped(Stop::TimeLimit));
        }
        self.write_cases(batch)?;
        let root = self.workspace.root();
        let stderr_path = root.join(format!("stderr-{}.log", batch.round));
        let stderr = File::create(&stderr_path).map_err(RouterError::Workspace)?;
        let mut child = Command::new(self.options.program)
            .args(arguments(&self.options, root, batch))
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|source| RouterError::Spawn {
                // The error outlives the borrowed arguments, so it owns the path.
                program: self.options.program.to_path_buf(),
                source,
            })?;
        // Nothing is piped, so the wait returns once the child itself is reaped.
        let status = match process::wait_with_deadline(&mut child, left, 0, "claude") {
            Ok(output) => output.status,
            Err(WaitError::Timeout) => return Ok(stopped(Stop::TimeLimit)),
            Err(WaitError::Io(error)) => return Err(RouterError::Workspace(error)),
        };
        let result_path = root.join(format!("result-{}.json", batch.round));
        if !result_path.is_file() {
            return Err(RouterError::Failed {
                status: status.to_string(),
                stderr: tail(&stderr_path).map_err(RouterError::Workspace)?,
            });
        }
        let dto: ResultDto =
            json_file::read(&result_path, MAX_RESULT_BYTES).map_err(RouterError::Unreadable)?;
        Ok(result::read(dto, self.plugin, self.graders)?)
    }
}

/// A batch that ran nothing and stopped for `stop`.
fn stopped(stop: Stop) -> Ran {
    Ran {
        runs: std::collections::BTreeMap::new(),
        cost_usd: 0.0,
        stop: Some(stop),
    }
}

/// The eval directory of batch `round`, below the plugin's copy.
fn eval_dir(round: u32) -> String {
    format!("anneal-routing-{round}")
}

/// The arguments of `claude` for `batch` in the workspace at `root`: its runs per case, no
/// baseline arm, no case failing the exit code, at most its budget, nothing published, no
/// scaffold scripts.
pub(crate) fn arguments(options: &Options<'_>, root: &Path, batch: &Batch<'_>) -> Vec<OsString> {
    let round = batch.round;
    let mut args: Vec<OsString> = vec!["plugin".into(), "eval".into()];
    args.push(root.join("plugin").into_os_string());
    args.extend([
        "--eval-dir".into(),
        eval_dir(round).into(),
        "--json".into(),
        root.join(format!("result-{round}.json")).into_os_string(),
        "--output-dir".into(),
        root.join(format!("out-{round}")).into_os_string(),
        "--report".into(),
        root.join(format!("report-{round}.html")).into_os_string(),
        "--runs".into(),
        batch.runs.to_string().into(),
        "--ablation".into(),
        "none".into(),
        "--threshold".into(),
        "0".into(),
        "--max-cost-usd".into(),
        budget_arg(batch.budget_usd).into(),
        "--no-publish".into(),
        "--no-scaffold".into(),
        "--concurrency".into(),
        options.concurrency.to_string().into(),
    ]);
    if let Some(model) = options.model {
        args.extend(["--model".into(), model.into()]);
    }
    if options.trust_plugin {
        args.push("--trust-plugin".into());
    }
    args
}

/// `budget_usd` rounded down to the cent, so a batch never gets more than is left.
pub(crate) fn budget_arg(budget_usd: f64) -> String {
    format!("{:.2}", (budget_usd * 100.0).floor() / 100.0)
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
    use std::ffi::OsString;
    use std::path::Path;

    use super::{Options, arguments, budget_arg};
    use crate::eval::routing::real::drive::Batch;

    #[test]
    fn a_batch_runs_the_suite_once_per_round_with_the_budget_left() {
        let options = Options {
            program: Path::new("claude"),
            model: Some("haiku"),
            concurrency: 4,
            trust_plugin: true,
            time_limit: std::time::Duration::from_secs(60),
        };

        let root = Path::new("w");
        let batch = Batch {
            round: 2,
            cases: Vec::new(),
            runs: 3,
            budget_usd: 4.567,
        };

        let args = arguments(&options, root, &batch);

        let path = |name: &str| root.join(name).into_os_string();
        let mut expected: Vec<OsString> = ["plugin", "eval"].map(OsString::from).to_vec();
        expected.extend([
            path("plugin"),
            "--eval-dir".into(),
            "anneal-routing-2".into(),
            "--json".into(),
            path("result-2.json"),
            "--output-dir".into(),
            path("out-2"),
            "--report".into(),
            path("report-2.html"),
        ]);
        expected.extend(
            [
                "--runs",
                "3",
                "--ablation",
                "none",
                "--threshold",
                "0",
                "--max-cost-usd",
                "4.56",
                "--no-publish",
                "--no-scaffold",
                "--concurrency",
                "4",
                "--model",
                "haiku",
                "--trust-plugin",
            ]
            .map(OsString::from),
        );
        assert_eq!(args, expected);
    }

    #[test]
    fn the_budget_is_rounded_down_to_the_cent() {
        assert_eq!(
            [budget_arg(5.0), budget_arg(0.019), budget_arg(1.999)],
            ["5.00", "0.01", "1.99"]
        );
    }
}
