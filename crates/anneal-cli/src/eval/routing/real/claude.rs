//! The [`Router`] adapter over `claude plugin eval`.
//!
//! The plugin is copied into a fresh temporary workspace (`eval::real::workspace`). Each batch
//! writes its cases to `<copy>/anneal-routing-<round>/` and runs the CLI through
//! `eval::real::claude::invoke`, within what is left of the time limit; that module says what
//! a kill at the time limit leaves running, and why a killed batch's cost is a lower bound.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

use super::drive::{Batch, Router};
use super::result;
use super::suite::{Graders, case};
use super::{Ran, RouterError, Stop};
use crate::eval::real::claude::{Invocation, Invoked, Options, budget_arg, invoke, result_path};
use crate::eval::real::result as shared_result;
use crate::eval::real::workspace::Workspace;

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
    /// are judged by `graders`. The time limit starts now, before the copy, so `--timeout`
    /// bounds the whole run.
    ///
    /// # Errors
    /// [`RouterError::Workspace`] when the workspace cannot be made or the plugin copied.
    pub(crate) fn new(
        options: Options<'a>,
        plugin_dir: &Path,
        plugin: &'a str,
        graders: &'a Graders<'a>,
    ) -> Result<Self, RouterError> {
        let started = Instant::now();
        Ok(Self {
            options,
            workspace: Workspace::create(plugin_dir).map_err(RouterError::Workspace)?,
            plugin,
            graders,
            started,
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
            return Ok(stopped(Spend::Nothing));
        }
        self.write_cases(batch)?;
        let root = self.workspace.root();
        let tag = batch.round.to_string();
        let invocation = Invocation {
            program: self.options.program,
            args: &arguments(&self.options, root, batch),
            root,
            tag: &tag,
        };
        match invoke(&invocation, left)? {
            Invoked::TimedOut => Ok(stopped(Spend::Unknown)),
            Invoked::Result(value) => Ok(result::read(
                shared_result::parse(value)?,
                self.plugin,
                self.graders,
            )?),
        }
    }
}

/// What a batch stopped by the time limit spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spend {
    /// It launched nothing.
    Nothing,
    /// It was killed before it reported its cost.
    Unknown,
}

/// A batch stopped by the time limit, with no runs.
fn stopped(spend: Spend) -> Ran {
    Ran {
        runs: std::collections::BTreeMap::new(),
        cost_usd: 0.0,
        cost_is_lower_bound: spend == Spend::Unknown,
        counted_errors: 0,
        stop: Some(Stop::TimeLimit),
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
        result_path(root, &round.to_string()).into_os_string(),
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use std::time::Duration;

    use super::{ClaudeRouter, Options, arguments};
    use crate::eval::routing::real::Stop;
    use crate::eval::routing::real::drive::{Batch, BatchCase, Router};
    use crate::eval::routing::real::suite::Graders;

    #[test]
    fn a_batch_runs_the_suite_once_per_round_with_the_budget_left() {
        let options = Options {
            program: Path::new("claude"),
            model: Some("haiku"),
            concurrency: 4,
            trust_plugin: true,
            time_limit: Duration::from_secs(60),
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
    fn with_no_time_left_a_batch_launches_nothing_and_its_spend_is_exact()
    -> Result<(), Box<dyn std::error::Error>> {
        let plugin = tempfile::tempdir()?;
        std::fs::write(plugin.path().join("README.md"), "plugin")?;
        let skills = vec!["plan".to_owned()];
        let graders = Graders::new("fixture", &skills);
        let options = Options {
            // Never started: no time is left.
            program: Path::new("no-such-claude"),
            model: None,
            concurrency: 1,
            trust_plugin: false,
            time_limit: Duration::ZERO,
        };
        let batch = Batch {
            round: 1,
            cases: vec![BatchCase {
                name: "prompt-001".to_owned(),
                prompt: "plan it",
            }],
            runs: 3,
            budget_usd: 5.0,
        };
        let mut router = ClaudeRouter::new(options, plugin.path(), "fixture", &graders)?;

        let ran = router.run(&batch);
        router.finish()?;

        let ran = ran?;
        assert_eq!(
            (
                ran.runs.len(),
                ran.cost_usd,
                ran.cost_is_lower_bound,
                ran.stop
            ),
            (0, 0.0, false, Some(Stop::TimeLimit))
        );
        Ok(())
    }
}
