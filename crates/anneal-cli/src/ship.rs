//! `anneal ship skip-rule [--base <ref>] [--head <rev>]`: whether a change may skip full
//! review.
//!
//! It reads `git diff --numstat` of `<base>...<head>` (base `--base`, else `origin/main`, else
//! `main`; head `--head`, default `HEAD`) and applies [`skip_rule`]: at most 2 files, under 50
//! changed lines, no binary file and no path matching a sensitive glob (the defaults, plus
//! `ship.sensitive_globs` from `.anneal/config.json`). Exit 0 skips full review, 1 needs it.
//! The merge gate applies the same rule through `skip`. See `docs/verdict.md`.

pub mod skip_rule;

use std::io::Write;
use std::path::Path;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths::{self, GIT_TIMEOUT, PathError};
use crate::pr_diff::{self, Budget, PrError};
use crate::repo_config;
use skip_rule::{Reason, Sensitive, SkipDecision};

/// `anneal ship`.
#[derive(Debug, Args)]
pub struct ShipArgs {
    #[command(subcommand)]
    action: ShipAction,
}

/// The `anneal ship` subcommands.
#[derive(Debug, Subcommand)]
pub enum ShipAction {
    /// Decide whether the change from the base may skip full review
    SkipRule(SkipRuleArgs),
}

/// `anneal ship skip-rule`.
#[derive(Debug, Args)]
pub struct SkipRuleArgs {
    /// The base the change is diffed from [default: origin/main, else main]
    #[arg(long, value_name = "REF")]
    base: Option<String>,
    /// The head of the change
    #[arg(long, value_name = "REV", default_value = "HEAD")]
    head: String,
}

/// The skip-rule applied to one change.
#[derive(Debug)]
pub(crate) struct Skip {
    /// The base, as given or defaulted.
    pub(crate) base: String,
    /// The rule's answer.
    pub(crate) decision: SkipDecision,
}

/// The change the skip-rule judges: `git diff <base>...<head>` in a worktree.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Change<'a> {
    /// The worktree's root.
    pub(crate) root: &'a Path,
    /// The base, a ref (`None`: the default base).
    pub(crate) base: Option<&'a str>,
    /// The head, a commit.
    pub(crate) head: &'a str,
}

/// Applies the skip-rule to `change`, with the sensitive `rules`
/// ([`skip_rule::sensitive_rules`]).
///
/// # Errors
/// The [`PrError`] when the base names no commit, git fails or prints what it does not
/// document.
pub(crate) fn skip(
    change: Change<'_>,
    rules: &[Sensitive],
    budget: Budget,
) -> Result<Skip, PrError> {
    let Change { root, base, head } = change;
    let base = match base {
        Some(base) => base.to_owned(),
        None => pr_diff::default_base(root, budget.next()?)?
            .ok_or(PrError::NoBase)?
            .to_owned(),
    };
    let base_commit = pr_diff::resolve_commit(root, &base, budget.next()?)?.ok_or_else(|| {
        PrError::UnknownBase {
            // The error owns the base it names.
            base: base.clone(),
        }
    })?;
    let output = pr_diff::numstat(root, &base_commit, head, budget.next()?)?;
    let changes = skip_rule::parse_numstat(&output)?;
    Ok(Skip {
        base,
        decision: skip_rule::judge(&changes, rules),
    })
}

/// Runs `anneal ship <action>` in `cwd`.
pub(crate) fn run<O: Write>(args: &ShipArgs, cwd: &Path, out: &mut O) -> Result<Code, CliError> {
    let ShipAction::SkipRule(args) = &args.action;
    let root = paths::worktree_root_within(cwd, GIT_TIMEOUT).map_err(|error| match error {
        PathError::NotARepository { .. } => CliError::Pr(PrError::NotAWorktree {
            command: "ship",
            source: error,
        }),
        other => CliError::Path(other),
    })?;
    let root = Path::new(&root);
    let config = repo_config::gate(root).map_err(PrError::from)?;
    let budget = Budget::per_call(GIT_TIMEOUT);
    let head = pr_diff::resolve_commit(root, &args.head, budget.next().map_err(PrError::from)?)
        .map_err(PrError::from)?
        .ok_or_else(|| PrError::UnknownCommit {
            flag: "--head",
            rev: args.head.clone(),
        })?;
    let rules = skip_rule::sensitive_rules(config.sensitive_globs).map_err(PrError::DefaultGlob)?;
    let change = Change {
        root,
        base: args.base.as_deref(),
        head: &head,
    };
    let skip = skip(change, &rules, budget)?;
    emit(&skip, &head, out)
}

/// `data` of `ship skip-rule`.
#[derive(Debug, Serialize)]
struct SkipData<'a> {
    base: &'a str,
    head: &'a str,
    skip: bool,
    files: usize,
    lines: u64,
    reasons: &'a [Reason],
    /// Binary and sensitive files past the listing cap; present only when some were.
    #[serde(skip_serializing_if = "Option::is_none")]
    omitted: Option<usize>,
}

fn emit<O: Write>(skip: &Skip, head: &str, out: &mut O) -> Result<Code, CliError> {
    let decision = &skip.decision;
    let data = SkipData {
        base: &skip.base,
        head,
        skip: decision.skip(),
        files: decision.files,
        lines: decision.lines,
        reasons: &decision.reasons,
        omitted: (decision.omitted > 0).then_some(decision.omitted),
    };
    let (code, findings) = if decision.skip() {
        (exit_codes::SHIP_SKIP, Vec::new())
    } else {
        let finding = Finding::new(
            format!("ship/full-review:{}", skip.base),
            "the change needs a full review (see data.reasons). fix: have it reviewed and \
             record the review with `anneal verdict set`.",
        );
        (exit_codes::SHIP_FULL_REVIEW, vec![finding])
    };
    Envelope::new(CommandName::Ship, Some("skip-rule"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}
