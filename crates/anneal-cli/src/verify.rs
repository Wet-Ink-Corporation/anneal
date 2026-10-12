//! `anneal verify risk [--base <ref>]`: whether the `verify` skill runs its doubt review (Clef
//! D5).
//!
//! It reads the working tree's change the way `floor-guard` does (committed and uncommitted
//! changes against the merge base of `HEAD` and the base, `--base`, else `origin/main`, else
//! `main`, plus untracked files read as added; `floor_guard::change`), fires the deterministic
//! `triggers` (auth, payments or migrations paths, more than 400 changed lines, a file too
//! large to count), and asks Clef D5 (`score defect_risk` on `clef-flash`, `clef`). The
//! triggers apply whatever Clef says, so they hold with Clef off or unreachable.
//!
//! Exit 3 (skip the doubt review) only when no trigger fired and D5, in effect enforce, answered
//! `skip` with its row logged; exit 0 (run it) otherwise, shadow included, since the skill ran
//! the doubt review every time before D5. A change that cannot be read exits 2. See
//! `docs/decide.md`.

pub(crate) mod clef;
pub(crate) mod triggers;

use std::ffi::OsString;
use std::io::Write;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::decide::DecideEnv;
use crate::decide::clock::Clock;
use crate::decide::transport::Transport;
use crate::envelope::Envelope;
use crate::error::CliError;
use crate::error_chain::reason;
use crate::exit_codes::{self, Code};
use crate::floor_guard::FloorError;
use crate::floor_guard::change::{self, WorkingChange};
use crate::glob::GlobError;
use clef::ClefReport;
use triggers::{Trigger, Triggers};

/// `anneal verify`.
#[derive(Debug, Args)]
pub struct VerifyArgs {
    #[command(subcommand)]
    action: VerifyAction,
}

/// The `anneal verify` subcommands.
#[derive(Debug, Subcommand)]
pub enum VerifyAction {
    /// Decide whether the working tree's change goes to the doubt review (Clef D5 and its
    /// deterministic triggers)
    Risk(RiskArgs),
}

/// `anneal verify risk`.
#[derive(Debug, Args)]
pub struct RiskArgs {
    /// The base the change is diffed from, through its merge base with HEAD [default:
    /// origin/main, else main]
    #[arg(long, value_name = "REF")]
    base: Option<String>,
}

/// Why `anneal verify risk` could not judge the change.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// The change could not be read whole: not a repository, no base, a git failure. Shown in
    /// `verify risk`'s own words, not as `floor-guard`'s.
    #[error("verify risk: reading the change: {}", change_failure(.0))]
    Change(FloorError),
    /// A default sensitive glob does not compile (a test keeps them valid).
    #[error("verify risk: a default sensitive glob")]
    Glob(#[source] GlobError),
}

impl VerifyError {
    /// The documented exit code: 2 when the change cannot be read, 70 for a glob that does
    /// not compile.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Change(_) => exit_codes::VERIFY_TOOL_ERROR,
            Self::Glob(_) => exit_codes::INTERNAL,
        }
    }
}

/// What went wrong reading the change, in `verify risk`'s words: each of [`FloorError`]'s
/// reasons without its `floor-guard:` prefix, and the causes of the two that have one.
/// Exhaustive, so a new reason has to choose its wording.
fn change_failure(error: &FloorError) -> String {
    match error {
        FloorError::Git(source) => reason(source),
        FloorError::BaseNotFound { base } => format!("base {base} does not resolve to a commit"),
        FloorError::NoDefaultBase => {
            "no base: neither origin/main nor main resolves; pass --base <ref>".to_owned()
        }
        FloorError::NoMergeBase { base, stderr } => {
            format!("HEAD and {base} have no merge base: {stderr}")
        }
        FloorError::GitFailed { what, stderr } => format!("git {what} failed: {stderr}"),
        FloorError::TooLarge { what, max } => {
            format!("git {what} printed more than {max} bytes; refusing a partial check")
        }
        FloorError::Read { path, source } => {
            format!("reading {}: {}", path.display(), reason(source))
        }
    }
}

/// `data` of `verify risk`.
#[derive(Debug, Serialize)]
struct RiskData<'a> {
    base: &'a str,
    merge_base: &'a str,
    files: usize,
    lines: u64,
    /// Whether the skill runs the doubt review.
    doubt_review: bool,
    triggers: &'a [Trigger],
    /// File triggers past the listing cap; present only when some were.
    #[serde(skip_serializing_if = "Option::is_none")]
    omitted: Option<usize>,
    clef: &'a ClefReport,
}

/// Runs `anneal verify <action>` with `env`, writing the envelope to `out`.
pub(crate) fn run<O, L, T, C>(
    args: &VerifyArgs,
    env: &DecideEnv<'_, L, T, C>,
    out: &mut O,
) -> Result<Code, CliError>
where
    O: Write,
    L: Fn(&str) -> Option<OsString>,
    T: Transport,
    C: Clock,
{
    let VerifyAction::Risk(args) = &args.action;
    let change =
        change::working_change(env.cwd, args.base.as_deref()).map_err(VerifyError::Change)?;
    let rules = triggers::rules().map_err(VerifyError::Glob)?;
    let found = triggers::judge(&change.files, &rules);
    let clef = clef::ask(&change.files, found.lines, env);
    emit(&change, &found, &clef, out)
}

fn emit<O: Write>(
    change: &WorkingChange,
    found: &Triggers,
    clef: &ClefReport,
    out: &mut O,
) -> Result<Code, CliError> {
    let triggered = !found.fired.is_empty() || found.omitted > 0;
    let doubt_review = triggered || !clef.says_skip();
    let data = RiskData {
        base: &change.base,
        merge_base: &change.merge_base,
        files: change.files.len(),
        lines: found.lines,
        doubt_review,
        triggers: &found.fired,
        omitted: (found.omitted > 0).then_some(found.omitted),
        clef,
    };
    let code = if doubt_review {
        exit_codes::VERIFY_REVIEW
    } else {
        exit_codes::VERIFY_SKIP
    };
    Envelope::new(CommandName::Verify, Some("risk"), code, data)
        .emit(out)
        .map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
