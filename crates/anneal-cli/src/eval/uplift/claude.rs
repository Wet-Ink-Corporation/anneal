//! The [`Evaluator`] adapter over `claude plugin eval`, sharing Tier 2's process step
//! ([`invoke`]), result checks ([`result::check`]) and workspace ([`Workspace`]).
//!
//! The plugin is copied once into a fresh `ae-<hex>` workspace as `with/`. The
//! baseline of a skill is a second copy, `without-<skill>/`, made from the first with only
//! the skill's directory removed from the plugin's skills directory (named as `--skills`
//! names it), so its hooks and sibling skills stay; a `--baseline` plugin is copied as
//! `baseline/`. No copy keeps an `anneal-uplift-*` eval directory. Each batch writes its
//! cases below the copy it runs against and runs single-arm (`--ablation none`): the two arms
//! are two runs, not `claude plugin eval`'s own with-without ablation, which would strip the
//! whole plugin and its hooks. A batch grants the union of its cases' gated tools, so each
//! case withholds the ones it does not name, and `claude` runs with its `TMPDIR` in the
//! workspace, outside the directories whose writes the Linux eval sandbox denies.

mod libexec;
mod scaffold;
mod traces;
mod trials;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::case::{Grader, Tool};
use super::drive::{Arm, Batch, Evaluator, Ran};
use super::suites::Case;
use crate::eval::real::claude::{Invocation, Invoked, Options, budget_arg, invoke, result_path};
use crate::eval::real::result;
use crate::eval::real::workspace::{Limits, Workspace, copy_tree, name_len};
use crate::eval::real::{RouterError, Stop};
pub(crate) use libexec::{LauncherTarget, Placement};
pub(crate) use trials::trials;

/// The case schema version this writes for a case without a fixture (Claude Code reads up to
/// 1.x).
const SCHEMA_VERSION: &str = "1.0";
/// The case schema version a `context` block needs.
const SCHEMA_VERSION_CONTEXT: &str = "1.1";

/// A baseline plugin given with `--baseline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BaselinePlugin<'a> {
    /// Its directory.
    pub(crate) dir: &'a Path,
    /// Its manifest's name.
    pub(crate) name: &'a str,
}

/// The plugin under test, and the plugin to compare it with if one is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Plugins<'a> {
    /// The plugin's directory.
    pub(crate) dir: &'a Path,
    /// The name of its skills directory, from which a skill-removed copy removes the skill.
    pub(crate) skills_dir: &'a OsStr,
    /// Its manifest's name.
    pub(crate) name: &'a str,
    /// The `--baseline` plugin, if given.
    pub(crate) baseline: Option<BaselinePlugin<'a>>,
    /// The `anneal` binary every copy gets, or `None` when the launcher knows no target for
    /// this host.
    pub(crate) anneal: Option<Placement<'a>>,
}

/// The [`Evaluator`] that runs `claude plugin eval` in a temporary workspace.
#[derive(Debug)]
pub(crate) struct ClaudeEvaluator<'a> {
    options: Options<'a>,
    workspace: Workspace,
    skills_dir: &'a OsStr,
    plugin: &'a str,
    baseline: Option<&'a str>,
    started: Instant,
    /// Where each batch's result is copied (`--keep-results`), if anywhere.
    keep_results: Option<&'a Path>,
    /// The results that could not be copied there, in batch order.
    unkept: Vec<Unkept>,
}

/// A batch result `--keep-results` could not keep. The batch's runs and cost still count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unkept {
    /// The file it would have been ([`kept_name`]).
    pub(crate) name: String,
    /// Why the copy failed.
    pub(crate) reason: String,
}

impl<'a> ClaudeEvaluator<'a> {
    /// An evaluator over a fresh copy of `plugins`' plugin, and of its baseline plugin if
    /// given, in a workspace made in `work_dir`, neither copy holding an eval directory and
    /// each holding `plugins.anneal`'s binary if given, with the [`CHILD_TMP`] directory each
    /// batch's `claude` gets as its `TMPDIR`. The time limit starts now, before the copies, so
    /// `--timeout` bounds the whole run.
    ///
    /// # Errors
    /// [`RouterError::Workspace`] when the workspace cannot be made, a plugin copied or the
    /// binary placed (the workspace is removed again).
    pub(crate) fn new(
        options: Options<'a>,
        plugins: Plugins<'a>,
        work_dir: &Path,
    ) -> Result<Self, RouterError> {
        let started = Instant::now();
        let workspace =
            Workspace::create_with_copy(work_dir, WORKSPACE_PREFIX, WITH_COPY, plugins.dir)
                .map_err(RouterError::Workspace)?;
        if let Err(error) = prepare(
            &workspace,
            plugins.baseline.map(|base| base.dir),
            plugins.anneal,
        ) {
            return Err(match workspace.finish() {
                Ok(()) => RouterError::Workspace(error),
                Err(removing) => RouterError::Cleanup {
                    error: Box::new(RouterError::Workspace(error)),
                    code: crate::exit_codes::INTERNAL,
                    removing: Box::new(RouterError::Workspace(removing)),
                },
            });
        }
        Ok(Self {
            options,
            workspace,
            skills_dir: plugins.skills_dir,
            plugin: plugins.name,
            baseline: plugins.baseline.map(|base| base.name),
            started,
            keep_results: None,
            unkept: Vec::new(),
        })
    }

    /// The evaluator, copying each batch's result into the existing directory `dir`, if
    /// given, as [`kept_name`] names it, and each of its runs' traces as
    /// [`traces::trace_name`] names them.
    pub(crate) const fn keeping_results(mut self, dir: Option<&'a Path>) -> Self {
        self.keep_results = dir;
        self
    }

    /// The results and traces `--keep-results` could not keep so far, taken out of the
    /// evaluator.
    pub(crate) fn take_unkept(&mut self) -> Vec<Unkept> {
        std::mem::take(&mut self.unkept)
    }

    /// Whether batches pass `--keep-temp`, so their runs' traces are still there to copy: with
    /// `--keep-results`, on Linux, where the kept runs are inside the workspace and go with it
    /// (macOS keeps them under `/tmp`, which nothing would remove).
    fn keeps_runs(&self) -> bool {
        self.keep_results.is_some() && cfg!(target_os = "linux")
    }

    /// Copies `batch`'s `result`, and its runs' `traces`, into the `--keep-results` directory,
    /// if given, recording a failed copy rather than failing a batch whose runs already
    /// happened and cost.
    fn keep(&mut self, result: &Path, batch: &Batch<'_>, traces: &[traces::Trace]) {
        let Some(dir) = self.keep_results else {
            return;
        };
        let name = kept_name(batch);
        if let Err(error) = fs::copy(result, dir.join(&name)) {
            self.unkept.push(Unkept {
                name,
                reason: error.to_string(),
            });
        }
        self.unkept
            .extend(traces::keep(traces, self.workspace.root(), dir));
    }

    /// What is left of the time limit.
    fn time_left(&self) -> Duration {
        self.options
            .time_limit
            .saturating_sub(self.started.elapsed())
    }

    /// Removes the workspace.
    ///
    /// # Errors
    /// [`RouterError::Workspace`] when it cannot be removed.
    pub(crate) fn finish(self) -> Result<(), RouterError> {
        self.workspace.finish().map_err(RouterError::Workspace)
    }

    /// The plugin copy `arm` runs against, made on first use for a skill's baseline.
    ///
    /// # Errors
    /// [`RouterError::Workspace`] when the copy cannot be made, or a skill name is not one
    /// plain path component.
    pub(crate) fn target(&mut self, arm: Arm<'_>) -> Result<PathBuf, RouterError> {
        match arm {
            Arm::With => Ok(self.workspace.plugin()),
            Arm::Baseline => Ok(self.workspace.root().join("baseline")),
            Arm::Without(skill) => {
                let mut parts = Path::new(skill).components();
                if !matches!(
                    (parts.next(), parts.next()),
                    (Some(Component::Normal(_)), None)
                ) {
                    return Err(RouterError::Workspace(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("skill name {skill:?} is not one path component"),
                    )));
                }
                let copy = self.workspace.root().join(format!("without-{skill}"));
                if !copy.exists() {
                    // Copied from the plugin copy, it holds the `anneal` binary `prepare` put
                    // there.
                    copy_tree(&self.workspace.plugin(), &copy, Limits::PLUGIN)
                        .and_then(|()| fs::remove_dir_all(copy.join(self.skills_dir).join(skill)))
                        .and_then(|()| remove_eval_dirs(&copy))
                        .map_err(RouterError::Workspace)?;
                }
                Ok(copy)
            }
        }
    }

    /// The name the result lists the plugin of `arm` under.
    fn plugin_of(&self, arm: Arm<'_>) -> &'a str {
        match (arm, self.baseline) {
            (Arm::Baseline, Some(name)) => name,
            (Arm::With | Arm::Without(_) | Arm::Baseline, _) => self.plugin,
        }
    }
}

impl Evaluator for ClaudeEvaluator<'_> {
    fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError> {
        if self.time_left().is_zero() {
            return Ok(stopped(false));
        }
        let target = self.target(batch.arm)?;
        write_cases(&target.join(eval_dir(batch.index)), batch)?;
        // The copy took time too: `claude` gets only what is left after it.
        let left = self.time_left();
        if left.is_zero() {
            return Ok(stopped(false));
        }
        let root = self.workspace.root();
        let tag = batch.index.to_string();
        let tmp_dir = root.join(CHILD_TMP);
        let mut args = arguments(&self.options, root, &target, batch);
        if self.keeps_runs() {
            args.push("--keep-temp".into());
        }
        let invocation = Invocation {
            program: self.options.program,
            args: &args,
            root,
            tag: &tag,
            tmp_dir: Some(&tmp_dir),
        };
        let invoked = invoke(&invocation, left)?;
        let result_path = invocation.result_path();
        match invoked {
            Invoked::TimedOut => Ok(stopped(true)),
            Invoked::Result(value) => {
                let result = result::parse(value)?;
                // Only kept runs leave their traces to copy.
                let traces = if self.keeps_runs() {
                    traces::traces(&result, batch)
                } else {
                    Vec::new()
                };
                let ran = trials(result, self.plugin_of(batch.arm), &batch.cases)?;
                self.keep(&result_path, batch, &traces);
                Ok(ran)
            }
        }
    }
}

/// The name `--keep-results` gives `batch`'s result: `result-<batch>-<arm>.json`
/// ([`batch_label`]).
fn kept_name(batch: &Batch<'_>) -> String {
    format!("result-{}.json", batch_label(batch))
}

/// `batch`'s index and arm, `<index>-<arm>`, the arm `with`, `without-<skill>` or `baseline`.
fn batch_label(batch: &Batch<'_>) -> String {
    let index = batch.index;
    match batch.arm {
        Arm::With => format!("{index}-with"),
        Arm::Without(skill) => format!("{index}-without-{skill}"),
        Arm::Baseline => format!("{index}-baseline"),
    }
}

/// A batch stopped by the time limit, with no runs; `killed` when it was killed before it
/// reported its cost.
fn stopped(killed: bool) -> Ran {
    Ran {
        runs: BTreeMap::new(),
        cost_usd: 0.0,
        cost_is_lower_bound: killed,
        stop: Some(Stop::TimeLimit),
        refused: None,
    }
}

/// The prefix of every batch's eval directory.
const EVAL_DIR_PREFIX: &str = "anneal-uplift-";

/// The workspace's directory each batch's `claude` gets as its `TMPDIR`, where
/// `claude plugin eval` makes its runs' sandbox roots: inside the work directory rather than
/// under the system's `/tmp`, whose writes the Linux eval sandbox denies (`super::sandbox`),
/// and removed with the workspace.
///
/// One byte, as every byte of the children's `TMPDIR` counts against the sandbox's socket
/// paths (`super::sandbox`).
const CHILD_TMP: &str = "t";

/// The prefix of the workspace's name, `ae-<6 hex>`: short for the same reason as
/// [`CHILD_TMP`].
const WORKSPACE_PREFIX: &str = "ae-";

/// The name of the plugin copy the with arm runs against, beside `without-<skill>/` and
/// `baseline/`. Not `plugin`: the anneal repository has `plugin/` at its root, so a run that
/// reads `${CLAUDE_PLUGIN_ROOT}` would take the workspace for that repository.
const WITH_COPY: &str = "with";

/// What the children's `TMPDIR` adds to the work directory's path, in bytes: `/ae-<hex>/t`,
/// 12, the same for every workspace.
pub(crate) const CHILD_TMP_BELOW_WORK_DIR: usize =
    1 + name_len(WORKSPACE_PREFIX) + 1 + CHILD_TMP.len();

/// The eval directory of batch `index`, below the plugin copy it runs against.
fn eval_dir(index: u32) -> String {
    format!("{EVAL_DIR_PREFIX}{index}")
}

/// Leaves the fresh `workspace`'s plugin copy, and a copy of the baseline plugin at
/// `baseline` if given, holding the plugins and no eval directory, so no batch runs cases
/// nobody selected, and the `anneal` binary if given; and makes the children's [`CHILD_TMP`].
fn prepare(
    workspace: &Workspace,
    baseline: Option<&Path>,
    anneal: Option<Placement<'_>>,
) -> io::Result<()> {
    fs::create_dir(workspace.root().join(CHILD_TMP))?;
    let plugin = workspace.plugin();
    remove_eval_dirs(&plugin)?;
    place(anneal, &plugin)?;
    if let Some(dir) = baseline {
        let copy = workspace.root().join("baseline");
        copy_tree(dir, &copy, Limits::PLUGIN)?;
        remove_eval_dirs(&copy)?;
        place(anneal, &copy)?;
    }
    Ok(())
}

/// Places `anneal`'s binary, if given, in the plugin copy at `copy`.
fn place(anneal: Option<Placement<'_>>, copy: &Path) -> io::Result<()> {
    anneal.map_or(Ok(()), |placement| placement.place_in(copy))
}

/// Removes from the plugin copy at `copy` any eval directory: one a source plugin already
/// held, or one an earlier batch wrote into the copy it was made from, so the copy holds the
/// plugin and nothing else.
fn remove_eval_dirs(copy: &Path) -> io::Result<()> {
    for entry in fs::read_dir(copy)? {
        let entry = entry?;
        let is_eval_dir = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(EVAL_DIR_PREFIX));
        if is_eval_dir && entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

/// The gated tools `batch` grants with `--allow-tools`: every one its cases name.
/// `claude plugin eval` grants them to every case of the batch, so [`write_cases`] has each
/// case withhold those it does not name.
fn granted(batch: &Batch<'_>) -> BTreeSet<Tool> {
    batch
        .cases
        .iter()
        .flat_map(|case| case.body.allowed_tools.iter().copied())
        .filter(|tool| tool.is_gated())
        .collect()
}

/// Writes each case of `batch` to `<dir>/<case>/case.yaml`, as JSON, withholding the gated
/// tools the batch grants that the case does not name, with the scaffold script beside it
/// ([`scaffold::SCRIPT_FILE`]) for a case seeded from a fixture.
fn write_cases(dir: &Path, batch: &Batch<'_>) -> Result<(), RouterError> {
    let granted = granted(batch);
    for case in &batch.cases {
        let case_dir = dir.join(case.name());
        fs::create_dir_all(&case_dir).map_err(RouterError::Workspace)?;
        let withheld = granted
            .iter()
            .copied()
            .filter(|tool| !case.body.allowed_tools.contains(tool))
            .collect();
        let text = serde_json::to_vec_pretty(&case_file(case, batch.runs).withholding(withheld))
            .map_err(|error| RouterError::Workspace(io::Error::other(error)))?;
        fs::write(case_dir.join("case.yaml"), text).map_err(RouterError::Workspace)?;
        if let Some(fixture) = &case.fixture {
            scaffold::write(&case_dir, &fixture.dir).map_err(RouterError::Workspace)?;
        }
    }
    Ok(())
}

/// The arguments of `claude` for `batch` against the plugin copy `target` in the workspace at
/// `root`: single-arm, no case failing the exit code, at most its budget, the gated tools its
/// cases name granted, nothing published, and scaffold scripts run only when one of its cases
/// is seeded from a fixture.
pub(crate) fn arguments(
    options: &Options<'_>,
    root: &Path,
    target: &Path,
    batch: &Batch<'_>,
) -> Vec<OsString> {
    let index = batch.index;
    let mut args: Vec<OsString> = vec!["plugin".into(), "eval".into(), target.into()];
    args.extend([
        "--eval-dir".into(),
        eval_dir(index).into(),
        "--json".into(),
        result_path(root, &index.to_string()).into_os_string(),
        "--output-dir".into(),
        root.join(format!("out-{index}")).into_os_string(),
        "--report".into(),
        root.join(format!("report-{index}.html")).into_os_string(),
        "--runs".into(),
        batch.runs.to_string().into(),
        "--ablation".into(),
        "none".into(),
        "--threshold".into(),
        "0".into(),
        "--max-cost-usd".into(),
        budget_arg(batch.budget_usd).into(),
    ]);
    let gated = granted(batch);
    if !gated.is_empty() {
        args.push("--allow-tools".into());
        args.extend(gated.into_iter().map(|tool| tool.name().into()));
    }
    let seeded = batch.cases.iter().any(|case| case.fixture.is_some());
    args.extend([
        "--no-publish".into(),
        if seeded {
            "--scaffold"
        } else {
            "--no-scaffold"
        }
        .into(),
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

/// One case as `case.yaml` holds it.
#[derive(Debug, Serialize)]
pub(crate) struct CaseFile<'a> {
    schema_version: &'static str,
    name: String,
    description: String,
    tags: [&'static str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<Context>,
    execution: Execution<'a>,
    runs: u32,
    graders: &'a [Grader],
}

/// What a run starts with besides the prompt: a seeded case's scaffold script.
#[derive(Debug, Serialize)]
struct Context {
    scaffold_script: &'static str,
}

#[derive(Debug, Serialize)]
struct Execution<'a> {
    prompt: &'a str,
    max_turns: u32,
    timeout_seconds: u32,
    allowed_tools: &'a [Tool],
    /// Gated tools the batch's `--allow-tools` grants that this case does not name, which
    /// `claude plugin eval` then removes from this case's runs alone.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    disallowed_tools: Vec<Tool>,
}

impl CaseFile<'_> {
    /// The case file with `tools` withheld from its runs (`execution.disallowed_tools`).
    pub(crate) fn withholding(mut self, tools: Vec<Tool>) -> Self {
        self.execution.disallowed_tools = tools;
        self
    }
}

/// `case` as `claude plugin eval` reads it, run `runs` times; a case seeded from a fixture
/// names its scaffold script, which needs schema 1.1.
pub(crate) fn case_file(case: &Case, runs: u32) -> CaseFile<'_> {
    let context = case.fixture.as_ref().map(|_| Context {
        scaffold_script: scaffold::SCRIPT_FILE,
    });
    CaseFile {
        schema_version: if context.is_some() {
            SCHEMA_VERSION_CONTEXT
        } else {
            SCHEMA_VERSION
        },
        name: case.name(),
        description: format!("anneal eval uplift (Tier 3): {}", case.file),
        tags: ["anneal-uplift"],
        context,
        execution: Execution {
            prompt: &case.body.prompt,
            max_turns: case.body.max_turns,
            timeout_seconds: case.body.timeout_seconds,
            allowed_tools: &case.body.allowed_tools,
            disallowed_tools: Vec::new(),
        },
        runs,
        graders: &case.body.graders,
    }
}

#[cfg(test)]
mod tests;
