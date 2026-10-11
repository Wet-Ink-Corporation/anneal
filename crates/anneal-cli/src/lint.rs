//! `anneal lint`: schema and anatomy checks whose every finding carries a rule, a location and
//! a fix string.
//!
//! - [`skills`]: `lint skills`, the 21 skill design rules.
//! - [`terms`]: `lint terms`, uses of the glossary's avoided terms; its hits are
//!   `{file, line, term}`, not rule findings, so it writes its own envelope `data`.
//! - [`spec`]: `lint spec`, a spec's sections, EARS criteria, markers, constitution check and
//!   avoided terms; its findings are [`spec::SpecFinding`]s (a spec rule, not a skill
//!   checker), so it writes its own envelope `data` too.
//! - [`plan`]: `lint plan`, a plan's spec link, sections, tasks, task fields and blockers;
//!   its findings are [`plan::PlanFinding`]s and its `data` carries the plan as read.
//! - [`finding`]: the finding model the skill rules use, and its envelope form.
//!
//! `lint brief` is a stub until its story lands. It will be a module beside [`skills`] with
//! its own `Args` type and `lint` function; [`run`] gets one more arm.

pub mod finding;
pub mod plan;
pub mod skills;
pub mod spec;
pub mod terms;

use std::io::Write;
use std::path::Path;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::Stub;
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::render::command_help;
use crate::exit_codes::{self, Code};
use finding::{LintFinding, Severity};
use skills::discover::{Capped, read_capped};
use skills::{LintError, require_file};

/// Arguments of `anneal lint`.
#[derive(Debug, Args)]
pub struct LintArgs {
    /// What to lint.
    #[command(subcommand)]
    pub target: LintTarget,
}

/// The things `anneal lint` checks.
#[derive(Debug, Subcommand)]
pub enum LintTarget {
    /// Check skill directories against the 21 skill design rules
    #[command(after_help = command_help(CommandName::Lint))]
    Skills(skills::SkillsArgs),
    /// Find the glossary's avoided terms (`_Avoid_:` lines) in files
    #[command(after_help = command_help(CommandName::Lint))]
    Terms(terms::TermsArgs),
    /// Check a spec: required sections, EARS acceptance criteria, Out of Scope, clarification
    /// markers, the constitution check and avoided terms
    #[command(after_help = command_help(CommandName::Lint))]
    Spec(spec::SpecArgs),
    /// Check a plan: the spec link, required sections, tasks with their files, test command
    /// and blockers
    #[command(after_help = command_help(CommandName::Lint))]
    Plan(plan::PlanArgs),
    /// Check a brief (not implemented yet)
    Brief(Stub),
}

/// Runs `anneal lint <target>` and writes its envelope to `out`.
///
/// # Errors
/// [`CliError::Lint`] when the target could not be checked, [`CliError::NotImplemented`] for a
/// stubbed target, [`CliError::Output`] when `out` cannot be written.
pub fn run<O: Write>(args: &LintArgs, out: &mut O) -> Result<Code, CliError> {
    match &args.target {
        LintTarget::Skills(skills_args) => {
            let outcome = skills::lint(skills_args)?;
            report(
                "skills",
                &outcome.findings,
                Counts::new(outcome.skills, outcome.cached),
                out,
            )
        }
        LintTarget::Terms(terms_args) => {
            let outcome = terms::lint(terms_args)?;
            terms::report(&terms_args.glossary, &outcome, out)
        }
        LintTarget::Spec(spec_args) => {
            let outcome = spec::lint(spec_args)?;
            spec::report(&outcome, out)
        }
        LintTarget::Plan(plan_args) => {
            let outcome = plan::lint(plan_args)?;
            plan::report(&outcome, out)
        }
        LintTarget::Brief(_) => Err(CliError::NotImplemented {
            command: CommandName::Lint,
        }),
    }
}

/// How many inputs a run looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Inputs checked (skills, specs, ...).
    checked: usize,
    /// Inputs whose findings were replayed from the snapshot.
    cached: usize,
}

impl Counts {
    /// `checked` inputs, `cached` of them replayed.
    #[must_use]
    pub(crate) const fn new(checked: usize, cached: usize) -> Self {
        Self { checked, cached }
    }
}

/// The envelope's `data`: counts by severity and every finding with its rule.
#[derive(Debug, Serialize)]
struct Data<'a> {
    checked: usize,
    cached: usize,
    errors: usize,
    warnings: usize,
    reviews: usize,
    details: Vec<Detail<'a>>,
}

#[derive(Debug, Serialize)]
struct Detail<'a> {
    key: String,
    rule: u8,
    #[serde(flatten)]
    finding: &'a LintFinding,
}

/// Writes the envelope for `findings` from the `lint` `target` (`"skills"`) and returns its
/// code: 1 when any finding is an error, else 0. Warnings and review items are listed but never
/// fail the run.
///
/// # Errors
/// [`CliError::Output`] when `out` cannot be written.
pub(crate) fn report<O: Write>(
    target: &'static str,
    findings: &[LintFinding],
    counts: Counts,
    out: &mut O,
) -> Result<Code, CliError> {
    let count = |severity| findings.iter().filter(|f| f.severity == severity).count();
    let errors = count(Severity::Error);
    let data = Data {
        checked: counts.checked,
        cached: counts.cached,
        errors,
        warnings: count(Severity::Warning),
        reviews: count(Severity::Review),
        details: findings
            .iter()
            .map(|finding| Detail {
                key: finding.key(),
                rule: finding.checker.rule(),
                finding,
            })
            .collect(),
    };
    let code = if errors > 0 {
        exit_codes::LINT_FINDINGS
    } else {
        exit_codes::LINT_CLEAN
    };
    let envelope_findings: Vec<Finding> = findings.iter().map(LintFinding::to_envelope).collect();
    Envelope::new(CommandName::Lint, Some(target), code, data)
        .with_findings(&envelope_findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// The contents of a file named on the command line, read up to the cap.
#[derive(Debug)]
pub(crate) enum Contents {
    /// Its text, lossily decoded.
    Text(String),
    /// Its size, over the cap.
    TooLarge(u64),
}

/// Reads a file named on the command line, up to the read cap. Text that is not UTF-8 is read
/// lossily.
///
/// # Errors
/// [`LintError::MissingPath`] or [`LintError::WrongKind`] when `path` is missing or a
/// directory, [`LintError::Io`] when it cannot be read.
pub(crate) fn read_named(path: &Path) -> Result<Contents, LintError> {
    require_file(path)?;
    let Capped { size, bytes } = read_capped(path)?;
    Ok(bytes.map_or(Contents::TooLarge(size), |bytes| {
        Contents::Text(String::from_utf8_lossy(&bytes).into_owned())
    }))
}

/// Reads a file named on the command line that must be read whole.
///
/// # Errors
/// As [`read_named`], plus [`LintError::TooLarge`] when it is over the read cap.
pub(crate) fn read_whole(path: &Path) -> Result<String, LintError> {
    read_whole_bytes(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The bytes of a file named on the command line that must be read whole, for callers that
/// hash exactly what is on disk (`plan approve|check`).
///
/// # Errors
/// As [`read_whole`].
pub(crate) fn read_whole_bytes(path: &Path) -> Result<Vec<u8>, LintError> {
    require_file(path)?;
    let Capped { size, bytes } = read_capped(path)?;
    bytes.ok_or_else(|| LintError::TooLarge {
        path: path.to_path_buf(),
        size,
    })
}
