//! `anneal eval`: evaluation suites.
//!
//! - [`routing`]: `eval routing`, Tier 1 lexical routing over `evals/routing/<skill>.yaml`.
//!
//! The envelope's `data` holds the counts, the confusion table as a map (`confusion`) and as
//! aligned text lines (`table`, for `jq -r '.data.table[]'`); every miss and every problem is a
//! finding.

pub mod routing;

use std::io::Write;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::Code;
use crate::exit_codes::render::command_help;
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
    /// Score routing fixtures against the skills by word overlap; 100% rank-1 or it fails
    #[command(after_help = command_help(CommandName::Eval))]
    Routing(routing::RoutingArgs),
}

/// Runs `anneal eval <suite>` and writes its envelope to `out`.
///
/// # Errors
/// [`CliError::Eval`] when the inputs could not be read, [`CliError::Output`] when `out`
/// cannot be written.
pub fn run<O: Write>(args: &EvalArgs, out: &mut O) -> Result<Code, CliError> {
    match &args.suite {
        EvalSuite::Routing(routing_args) => report(&routing::evaluate(routing_args)?, out),
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

fn problem_finding(problem: &Problem) -> Finding {
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
