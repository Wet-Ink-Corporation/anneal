//! `anneal eval`: evaluation suites.
//!
//! - [`routing`]: `eval routing`, Tier 1 lexical routing over `evals/routing/<skill>.yaml`,
//!   and with `--router claude` Tier 2, the same fixtures under Claude Code's own router
//!   ([`routing::real`]).
//! - [`uplift`]: `eval uplift`, Tier 3, each entry skill's regression and capability suites
//!   under `evals/uplift/` run with the skill and without it.
//!
//! Tiers 2 and 3 share `real`: the `claude plugin eval` driver, its result reader, the Wilson
//! interval and the temporary workspace.
//!
//! The envelope's `data` holds the counts, the confusion table as a map (`confusion`) and as
//! aligned text lines (`table`, for `jq -r '.data.table[]'`); every miss and every problem is a
//! finding.

mod real;
pub mod routing;
pub mod uplift;

use std::ffi::OsString;
use std::io::Write;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::Code;
use crate::exit_codes::render::command_help;
use routing::real::RealArgs;
use routing::{Case, CaseKind, Confusion, Outcome, Problem};

/// Arguments of `anneal eval`.
#[derive(Debug, Args)]
pub struct EvalArgs {
    /// Which suite to run.
    #[command(subcommand)]
    pub suite: EvalSuite,
}

/// The suites `anneal eval` runs.
#[derive(Debug, Subcommand)]
pub enum EvalSuite {
    /// Score routing fixtures against the skills: by word overlap (100% rank-1), or with
    /// `--router claude` under Claude Code's router (Wilson intervals; runs a model)
    #[command(after_help = command_help(CommandName::Eval))]
    Routing(RoutingCommand),
    /// Score each skill's behaviour uplift: its regression and capability suites run with the
    /// skill and without it under `claude plugin eval` (Newcombe intervals; runs a model)
    #[command(after_help = command_help(CommandName::Eval))]
    Uplift(uplift::UpliftArgs),
}

/// Arguments of `anneal eval routing`: the inputs both routers read, and the Tier 2 flags.
#[derive(Debug, Clone, Args)]
pub struct RoutingCommand {
    /// The skills and fixtures.
    #[command(flatten)]
    pub inputs: routing::RoutingArgs,
    /// The router and its Tier 2 settings.
    #[command(flatten)]
    pub real: RealArgs,
}

/// Runs `anneal eval <suite>` and writes its envelope to `out`; `lookup` reads one environment
/// variable (`eval uplift`'s default work directory).
///
/// # Errors
/// [`CliError::Usage`] for a Tier 2 flag without `--router claude` or a run flag with
/// `eval uplift --check`, [`CliError::Eval`] or [`CliError::Uplift`] when the inputs could not
/// be read or the real router could not run, [`CliError::Output`] when `out` cannot be
/// written.
pub fn run<O: Write>(
    args: &EvalArgs,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    match &args.suite {
        EvalSuite::Routing(command) => match command.real.settings()? {
            None => report(&routing::evaluate(&command.inputs)?, out),
            Some(settings) => {
                routing::real::report(&routing::real::evaluate(&command.inputs, &settings)?, out)
            }
        },
        EvalSuite::Uplift(args) => {
            let mode = args.mode()?;
            let fallback = uplift::default_work_dir(lookup);
            uplift::report(&uplift::evaluate(args, &mode, &fallback)?, out)
        }
    }
}

/// The envelope's `data` for `eval routing`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    skills: usize,
    cases: usize,
    passed: usize,
    failed: usize,
    problems: usize,
    confusion: &'a Confusion<'a>,
    table: Vec<String>,
}

fn report<O: Write>(outcome: &Outcome, out: &mut O) -> Result<Code, CliError> {
    let failed = outcome.misses().count();
    let confusion = outcome.confusion();
    let data = Data {
        skills: outcome.skills.len(),
        cases: outcome.cases.len(),
        passed: outcome.cases.len().saturating_sub(failed),
        failed,
        problems: outcome.problems.len(),
        confusion: &confusion,
        table: confusion.render(),
    };
    let findings: Vec<Finding> = outcome
        .problems
        .iter()
        .map(problem_finding)
        .chain(outcome.misses().map(miss_finding))
        .collect();
    Envelope::new(CommandName::Eval, Some("routing"), outcome.code(), data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// The finding of a fixture or skill problem, shared by both routers' reports.
pub(crate) fn problem_finding(problem: &Problem) -> Finding {
    match problem {
        Problem::Skill(finding) => Finding::new(
            format!("eval/routing/skill:{}", finding.key()),
            format!("{}: {}", finding.at, finding.message),
        ),
        Problem::MissingFixture { skill } => Finding::new(
            format!("eval/routing/missing-fixture:{skill}"),
            format!("model-invoked skill `{skill}` has no routing fixture `{skill}.yaml`"),
        ),
        Problem::Fixture { file, line, fault } => Finding::new(
            format!("eval/routing/fixture:{file}:{line}"),
            format!("{file}:{line}: {fault}"),
        ),
    }
}

fn miss_finding(case: &Case) -> Finding {
    let (kind, verdict) = match &case.kind {
        CaseKind::Positive => (
            "positive",
            format!(
                "positive for `{}` ranked `{}` first",
                case.expected,
                case.top.label()
            ),
        ),
        CaseKind::NearMiss { mimics } => (
            "near-miss",
            format!(
                "near miss owned by `{}` did not outrank `{mimics}`",
                case.expected
            ),
        ),
    };
    let rival = case.rival.as_ref().map_or_else(String::new, |rival| {
        format!(", {} {}", rival.skill, rival.score)
    });
    Finding::new(
        format!("eval/routing/{kind}:{}:{}", case.file, case.line),
        format!(
            "{verdict} ({} {}{rival}): {:?}",
            case.expected, case.expected_score, case.prompt
        ),
    )
}
