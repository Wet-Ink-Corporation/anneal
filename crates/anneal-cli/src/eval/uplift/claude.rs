//! The [`Evaluator`] adapter over `claude plugin eval`, sharing Tier 2's process step
//! ([`invoke`]), result checks ([`result::check`]) and workspace ([`Workspace`]).
//!
//! The plugin is copied once into a fresh `anneal-eval-uplift-*` workspace as `plugin/`. The
//! baseline of a skill is a second copy, `without-<skill>/`, made from the first with only
//! the skill's directory removed from the plugin's skills directory (named as `--skills`
//! names it), so its hooks and sibling skills stay; a `--baseline` plugin is copied as
//! `baseline/`. No copy keeps an `anneal-uplift-*` eval directory. Each batch writes its
//! cases below the copy it runs against and runs single-arm (`--ablation none`): the two arms
//! are two runs, not `claude plugin eval`'s own with-without ablation, which would strip the
//! whole plugin and its hooks.

mod libexec;
mod scaffold;
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
use crate::eval::real::workspace::{Limits, Workspace, copy_tree};
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
}

impl<'a> ClaudeEvaluator<'a> {
    /// An evaluator over a fresh copy of `plugins`' plugin, and of its baseline plugin if
    /// given, neither holding an eval directory and each holding `plugins.anneal`'s binary if
    /// given. The time limit starts now, before the copies, so `--timeout` bounds the whole
    /// run.
    ///
    /// # Errors
    /// [`RouterError::Workspace`] when the workspace cannot be made, a plugin copied or the
    /// binary placed (the workspace is removed again).
    pub(crate) fn new(options: Options<'a>, plugins: Plugins<'a>) -> Result<Self, RouterError> {
        let started = Instant::now();
        let workspace =
            Workspace::create_for("uplift", plugins.dir).map_err(RouterError::Workspace)?;
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
        })
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
        let invocation = Invocation {
            program: self.options.program,
            args: &arguments(&self.options, root, &target, batch),
            root,
            tag: &tag,
        };
        match invoke(&invocation, left)? {
            Invoked::TimedOut => Ok(stopped(true)),
            Invoked::Result(value) => Ok(trials(
                result::parse(value)?,
                self.plugin_of(batch.arm),
                &batch.cases,
            )?),
        }
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

/// The eval directory of batch `index`, below the plugin copy it runs against.
fn eval_dir(index: u32) -> String {
    format!("{EVAL_DIR_PREFIX}{index}")
}

/// Leaves the fresh `workspace`'s plugin copy, and a copy of the baseline plugin at
/// `baseline` if given, holding the plugins and no eval directory, so no batch runs cases
/// nobody selected, and the `anneal` binary if given.
fn prepare(
    workspace: &Workspace,
    baseline: Option<&Path>,
    anneal: Option<Placement<'_>>,
) -> io::Result<()> {
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

/// Writes each case of `batch` to `<dir>/<case>/case.yaml`, as JSON, with the scaffold script
/// beside it ([`scaffold::SCRIPT_FILE`]) for a case seeded from a fixture.
fn write_cases(dir: &Path, batch: &Batch<'_>) -> Result<(), RouterError> {
    for case in &batch.cases {
        let case_dir = dir.join(case.name());
        fs::create_dir_all(&case_dir).map_err(RouterError::Workspace)?;
        let text = serde_json::to_vec_pretty(&case_file(case, batch.runs))
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
    let gated: BTreeSet<Tool> = batch
        .cases
        .iter()
        .flat_map(|case| case.body.allowed_tools.iter().copied())
        .filter(|tool| tool.is_gated())
        .collect();
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
        },
        runs,
        graders: &case.body.graders,
    }
}

#[cfg(test)]
mod tests;
