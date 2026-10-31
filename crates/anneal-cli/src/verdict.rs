//! `anneal verdict set|valid`: review verdicts per pull request, in the ledger.
//!
//! `set --pr <n> --sha <rev> --level <tests-verified|reviewed|approved> --source
//! <local|review-plugin|human> [--actor human|model] [--via mod|bash|terminal] [--base <ref>]
//! [--session <id>] [--no-status]` appends one `verdict_set` with the full commit `--sha`
//! names, the base (`--base`, else `origin/main`, else `main`), the `git patch-id --verbatim`
//! of `git diff <base>...<sha>` and, when given, who ran it and through what (`by`, #62). It
//! has no `--store`: the verdict goes to the ledger the merge gate reads. Then, with a GitHub
//! token and without `--no-status`, it posts the `anneal/verdict` commit status on that commit
//! (#116); a status that was not posted exits 4 with the verdict still recorded.
//!
//! `valid --pr <n> [--head <rev>] [--base <ref>]` judges the pull request's verdicts against
//! `--head` (default `HEAD`) as `assess` describes: exit 0 when its latest fresh verdict is a
//! level, 3 when it is `noise_only`, 1 otherwise. It never creates a ledger. See
//! `docs/verdict.md`.
//!
//! - `assess`: freshness and the requirements, pure.
//! - `pull_request`: the verdicts read from the ledger with the head they are judged
//!   against, shared with the merge gate.
//! - `set`: `verdict set`, the append and then the status.
//! - `commit_status`: the `anneal/verdict` status: its state from the merge gate, the
//!   repository from `origin`, and the ureq adapter that posts it.

pub(crate) mod assess;
mod commit_status;
pub(crate) mod pull_request;
mod set;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anneal_ledger::event::{Verdict, VerdictSource};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use crate::command::CommandName;
use crate::decide::label::{Actor, Via};
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::existing_ledger::{self, LedgerError};
use crate::exit_codes::{self, Code};
use crate::paths::{self, GIT_TIMEOUT, PathError};
use crate::pr_diff::{Budget, PrError};
use crate::repo_config;
use assess::{Fresh, Invalid, Judged, Outcome};
pub(crate) use commit_status::GithubEnv;
use pull_request::{PullRequest, Query, ReadError};

/// How long `verdict valid` waits for another process's ledger lock, as `Store::open` does.
const LEDGER_BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// `anneal verdict`.
#[derive(Debug, Args)]
pub struct VerdictArgs {
    #[command(subcommand)]
    action: VerdictAction,
}

/// The `anneal verdict` subcommands.
#[derive(Debug, Subcommand)]
pub enum VerdictAction {
    /// Record a verdict for a pull request's head commit
    Set(SetArgs),
    /// Check that a pull request has a fresh verdict for its head
    Valid(ValidArgs),
}

/// `anneal verdict set`.
#[derive(Debug, Args)]
pub struct SetArgs {
    /// The pull request number
    #[arg(long, value_name = "N")]
    pr: u64,
    /// The head commit the verdict is for (any revision git resolves locally)
    #[arg(long, value_name = "REV")]
    sha: String,
    /// The level the change reached
    #[arg(long, value_enum)]
    level: Level,
    /// Who sets the verdict
    #[arg(long, value_enum)]
    source: Source,
    /// Who runs this command [default: none recorded, so no one is claimed]; the guard denies a
    /// model's `--actor human`
    #[arg(long, value_enum)]
    actor: Option<Actor>,
    /// Through what it is run (mod: a Claude Code mod's buttons) [default: none recorded]
    #[arg(long, value_enum)]
    via: Option<Via>,
    /// The base the patch id is taken from [default: origin/main, else main]
    #[arg(long, value_name = "REF")]
    base: Option<String>,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
    /// Post no anneal/verdict commit status [default: posted when a GitHub token is set]
    #[arg(long)]
    no_status: bool,
}

/// `anneal verdict valid`.
#[derive(Debug, Args)]
pub struct ValidArgs {
    /// The pull request number
    #[arg(long, value_name = "N")]
    pr: u64,
    /// The head to judge the verdicts against
    #[arg(long, value_name = "REV", default_value = "HEAD")]
    head: String,
    /// The base the head's patch id is taken from [default: the latest verdict's]
    #[arg(long, value_name = "REF")]
    base: Option<String>,
}

/// A level `verdict set` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Level {
    /// The change's tests were verified green.
    TestsVerified,
    /// The change was reviewed.
    Reviewed,
    /// The change may merge.
    Approved,
}

impl Level {
    /// The level as `--level` names it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TestsVerified => "tests-verified",
            Self::Reviewed => "reviewed",
            Self::Approved => "approved",
        }
    }
}

impl From<Level> for Verdict {
    fn from(level: Level) -> Self {
        match level {
            Level::TestsVerified => Self::TestsVerified,
            Level::Reviewed => Self::Reviewed,
            Level::Approved => Self::Approved,
        }
    }
}

/// Who sets a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Source {
    /// anneal's own checks, such as `verify`.
    Local,
    /// The review plugin.
    ReviewPlugin,
    /// A human.
    Human,
}

impl Source {
    /// The source as `--source` names it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::ReviewPlugin => "review-plugin",
            Self::Human => "human",
        }
    }
}

impl From<Source> for VerdictSource {
    fn from(source: Source) -> Self {
        match source {
            Source::Local => Self::Local,
            Source::ReviewPlugin => Self::ReviewPlugin,
            Source::Human => Self::Human,
        }
    }
}

/// What `anneal verdict` gets from the process: its working directory, its environment (for
/// the ledger), what the environment says about GitHub (read at the edge), and stdout and
/// stderr.
#[derive(Debug)]
pub(crate) struct Io<'a, L, O, E> {
    /// The working directory.
    pub(crate) cwd: &'a Path,
    /// The environment.
    pub(crate) lookup: L,
    /// The token and API override `verdict set` posts its commit status with.
    pub(crate) github: &'a GithubEnv,
    /// The envelope's stream.
    pub(crate) out: O,
    /// The stream for `verdict set`'s warning when its status is not posted.
    pub(crate) err: E,
}

/// Runs `anneal verdict <action>` with `io`.
pub(crate) fn run<L, O, E>(args: &VerdictArgs, io: Io<'_, L, O, E>) -> Result<Code, CliError>
where
    L: Fn(&str) -> Option<OsString>,
    O: Write,
    E: Write,
{
    let root = worktree_root(io.cwd)?;
    let root = Path::new(&root);
    match &args.action {
        VerdictAction::Set(set) => set::run(set, root, io),
        VerdictAction::Valid(valid) => {
            let Io {
                cwd,
                lookup,
                mut out,
                ..
            } = io;
            run_valid(valid, Place { cwd, root }, lookup, &mut out)
        }
    }
}

/// Where `verdict` runs: the working directory (the ledger is found from it) and its
/// worktree's root (git runs there).
#[derive(Debug, Clone, Copy)]
struct Place<'a> {
    cwd: &'a Path,
    root: &'a Path,
}

/// The worktree's root for `cwd`; outside one, a usage error.
fn worktree_root(cwd: &Path) -> Result<String, CliError> {
    paths::worktree_root_within(cwd, GIT_TIMEOUT).map_err(|error| match error {
        PathError::NotARepository { .. } => CliError::Pr(PrError::NotAWorktree {
            command: "verdict",
            source: error,
        }),
        other => CliError::Path(other),
    })
}

/// `anneal verdict valid`.
fn run_valid<O: Write>(
    args: &ValidArgs,
    place: Place<'_>,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let Place { cwd, root } = place;
    let config = repo_config::gate(root).map_err(PrError::from)?;
    let store = existing_ledger::open_existing(cwd, lookup, GIT_TIMEOUT, LEDGER_BUSY_TIMEOUT)
        .map_err(|error| match error {
            LedgerError::CurrentDir(source) => CliError::CurrentDir(source),
            LedgerError::Locate(source) => CliError::Path(source),
            LedgerError::Open(source) => CliError::Ledger(source),
        })?;
    let query = Query {
        root,
        pr: args.pr,
        head: ("--head", &args.head),
        base: args.base.as_deref(),
        budget: Budget::per_call(GIT_TIMEOUT),
    };
    let pull_request = pull_request::read(store.as_ref(), &query).map_err(read_error)?;
    let assessment = pull_request.assess(config.require_plugin);
    let outcome = assessment.outcome();
    let data = ValidData {
        pr: pull_request.pr,
        head: &pull_request.head,
        base: pull_request.base.as_deref(),
        patch_id: pull_request.patch_id.as_deref(),
        require_plugin: config.require_plugin,
        verdict: assessment.latest.map(Shown::from),
        invalid: match outcome {
            Outcome::Invalid(why) => Some(why),
            Outcome::Valid | Outcome::NoiseOnly => None,
        },
        met: Met {
            tests: assessment.tests.is_some(),
            review: assessment.review.is_some(),
        },
        stale: assessment.stale,
    };
    let (code, findings) = verdict_code(&pull_request, outcome);
    Envelope::new(CommandName::Verdict, Some("valid"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// The error for a failed read of the pull request.
fn read_error(error: ReadError) -> CliError {
    match error {
        ReadError::Pr(error) => CliError::Pr(error),
        ReadError::Ledger(error) => CliError::Ledger(error),
        ReadError::Tag(error) => CliError::Usage {
            reason: format!("--pr is recorded as a ledger tag: {error}"),
        },
    }
}

/// The exit code for `outcome`, and a finding naming what to do when it is not valid.
fn verdict_code(pull_request: &PullRequest, outcome: Outcome) -> (Code, Vec<Finding>) {
    let pr = pull_request.pr;
    let key = format!("verdict/invalid:{pr}");
    match outcome {
        Outcome::Valid => (exit_codes::VERDICT_VALID, Vec::new()),
        Outcome::NoiseOnly => (exit_codes::VERDICT_NOISE_ONLY, Vec::new()),
        Outcome::Invalid(why) => {
            let message = match why {
                Invalid::NoVerdict => format!(
                    "pull request {pr} has no verdict. fix: verify the change, then run \
                     `anneal verdict set --pr {pr} --sha HEAD --level tests-verified --source \
                     local`."
                ),
                Invalid::Stale => format!(
                    "every verdict of pull request {pr} is for other code (its head is {}). \
                     fix: verify the change again and set a verdict for the head.",
                    pull_request.head
                ),
                Invalid::ChangesRequested => format!(
                    "the latest verdict on pull request {pr} requests changes. fix: address \
                     them, then verify and set a verdict again."
                ),
            };
            (
                exit_codes::VERDICT_INVALID,
                vec![Finding::new(key, message)],
            )
        }
    }
}

/// `data` of `verdict valid`.
#[derive(Debug, Serialize)]
struct ValidData<'a> {
    pr: u64,
    head: &'a str,
    base: Option<&'a str>,
    patch_id: Option<&'a str>,
    require_plugin: bool,
    /// The latest fresh verdict.
    verdict: Option<Shown<'a>>,
    /// Why there is no valid verdict; absent when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    invalid: Option<Invalid>,
    /// Which of the merge gate's requirements fresh verdicts meet.
    #[serde(flatten)]
    met: Met,
    /// How many verdicts are for other code.
    stale: usize,
}

/// The merge gate's requirements a pull request's fresh verdicts meet, shown in `data` beside
/// the rest.
#[derive(Debug, Serialize)]
struct Met {
    /// A fresh level meets the tests requirement.
    tests: bool,
    /// A fresh review verdict meets the review requirement.
    review: bool,
}

/// A fresh verdict as shown in `data`.
#[derive(Debug, Serialize)]
struct Shown<'a> {
    position: u64,
    verdict: Verdict,
    source: Option<VerdictSource>,
    sha: Option<&'a str>,
    fresh: Fresh,
}

impl<'a> From<Judged<'a>> for Shown<'a> {
    fn from(judged: Judged<'a>) -> Self {
        let set = judged.record.set;
        Self {
            position: judged.record.position,
            verdict: set.verdict,
            source: set.source,
            sha: set.sha.as_deref(),
            fresh: judged.fresh,
        }
    }
}
