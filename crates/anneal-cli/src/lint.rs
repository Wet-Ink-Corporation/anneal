//! `anneal lint`: schema and anatomy checks whose every finding carries a rule, a location and
//! a fix string.
//!
//! - [`skills`]: `lint skills`, the 21 skill design rules.
//! - [`finding`]: the finding model every target shares, and its envelope form.
//!
//! `lint spec|plan|brief` are stubs until their stories land. Each will be a module beside
//! [`skills`] with its own `Args` type and `lint` function returning findings; [`run`] gets
//! one more arm and reuses [`report`] unchanged.

pub mod finding;
pub mod skills;

use std::io::Write;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::Stub;
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::render::command_help;
use crate::exit_codes::{self, Code};
use finding::{LintFinding, Severity};

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
    /// Check a spec (not implemented yet)
    Spec(Stub),
    /// Check a plan (not implemented yet)
    Plan(Stub),
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
        LintTarget::Spec(_) | LintTarget::Plan(_) | LintTarget::Brief(_) => {
            Err(CliError::NotImplemented {
                command: CommandName::Lint,
            })
        }
    }
}

/// How many inputs a run looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Inputs checked (skills, specs, ...).
    pub checked: usize,
    /// Inputs whose findings were replayed from the snapshot.
    pub cached: usize,
}

impl Counts {
    /// `checked` inputs, `cached` of them replayed.
    #[must_use]
    pub const fn new(checked: usize, cached: usize) -> Self {
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
pub fn report<O: Write>(
    target: &str,
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
