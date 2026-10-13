//! `anneal watch pr|ci|deploy`: long-poll a pull request, a commit's checks or a deployment
//! until it settles or the timeout passes, and exit with one code per blocker.
//!
//! `pr <n>` reads a pull request of gh's current repository (no `--repo`: the merge gate it
//! ends with reads this worktree's ledger); `ci <ref> [--repo]` the checks of the commit a
//! branch, tag or commit names; `deploy <id> [--provider github] [--repo]` a deployment's
//! latest status. Each polls with back-off (10 s, doubling, at most 60 s) until a blocker
//! appears or nothing is pending, at most `--timeout` seconds (default 1800; 0 polls once).
//!
//! Exit codes: 0 clear, 2 merge conflict, 3 unresolved review threads, 4 failing checks (or a
//! failed deployment), 5 timeout, 6 the merge gate would deny the head, 7 query error (one
//! `ERROR` line, nothing on stdout). With several blockers the lowest code wins; the envelope
//! names every one (`data.blockers`, one finding each), and on a timeout what was still
//! pending. GitHub is read only through `gh`: REST `GET`, and one GraphQL query for review
//! threads. Ctrl-C ends the process by the default SIGINT action: no handler, no temporary
//! file. See `docs/watch.md`.
//!
//! - `poll`: the loop, the back-off and the `Clock` port.
//! - `snapshot`: one poll's blockers and pending work, and their exit codes.
//! - `github`: the pull request and commit readers (REST).
//! - `threads`: a pull request's unresolved review threads (GraphQL, or the Claude Code
//!   proxy's route where GraphQL is refused).
//! - `deploy`: the `DeployProvider` port and its GitHub Deployments reader.
//! - `gate`: the merge gate (`hook::merge_gate::judge_head`) behind the `HeadGate` port.

pub(crate) mod deploy;
#[cfg(test)]
mod fake;
pub(crate) mod gate;
pub(crate) mod github;
pub(crate) mod poll;
pub(crate) mod snapshot;
pub(crate) mod threads;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use crate::command::CommandName;
use crate::dag::tracker::github::{GhApi, GhCli, GhError, GhGraphql, Repo};
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::render::command_help;
use crate::exit_codes::{self, Code};
use deploy::{DeployWatch, GitHubDeployments};
use gate::{HeadGate, LedgerGate};
use github::{CommitWatch, PullRequestWatch, RepoRef};
use poll::{Clock, DEFAULT_TIMEOUT_SECS, Schedule, SystemClock, Watched};
use snapshot::{Blocker, Pending};

/// `anneal watch`.
#[derive(Debug, Args)]
pub struct WatchArgs {
    #[command(subcommand)]
    action: WatchAction,
}

/// The `anneal watch` subcommands.
#[derive(Debug, Subcommand)]
pub enum WatchAction {
    /// Watch a pull request of this repository: conflicts, review threads, checks, then the
    /// merge gate on its head
    #[command(after_help = command_help(CommandName::Watch))]
    Pr(PrArgs),
    /// Watch the checks of the commit a branch, tag or commit names
    #[command(after_help = command_help(CommandName::Watch))]
    Ci(CiArgs),
    /// Watch a deployment's latest status
    #[command(after_help = command_help(CommandName::Watch))]
    Deploy(DeployArgs),
}

/// `anneal watch pr`.
#[derive(Debug, Args)]
pub struct PrArgs {
    /// The pull request number
    #[arg(value_name = "N")]
    pr: u64,
    #[command(flatten)]
    wait: Wait,
}

/// `anneal watch ci`.
#[derive(Debug, Args)]
pub struct CiArgs {
    /// The branch, tag or commit whose checks to watch
    #[arg(value_name = "REF")]
    git_ref: String,
    /// The repository, as owner/name [default: gh's repository for this directory]
    #[arg(long, value_name = "OWNER/NAME")]
    repo: Option<Repo>,
    #[command(flatten)]
    wait: Wait,
}

/// `anneal watch deploy`.
#[derive(Debug, Args)]
pub struct DeployArgs {
    /// The deployment id (for GitHub, the number `gh api repos/{owner}/{repo}/deployments`
    /// lists)
    #[arg(value_name = "ID")]
    id: String,
    /// Where the deployment's status is read
    #[arg(long, value_enum, default_value_t = Provider::Github)]
    provider: Provider,
    /// The repository, as owner/name [default: gh's repository for this directory]
    #[arg(long, value_name = "OWNER/NAME")]
    repo: Option<Repo>,
    #[command(flatten)]
    wait: Wait,
}

/// How long to wait.
#[derive(Debug, Clone, Copy, Args)]
pub struct Wait {
    /// Give up after this many seconds, exiting 5 (0 polls once)
    #[arg(long, value_name = "SECS", default_value_t = DEFAULT_TIMEOUT_SECS)]
    timeout: u64,
}

impl Wait {
    const fn schedule(self) -> Schedule {
        Schedule::with_timeout(Duration::from_secs(self.timeout))
    }
}

/// The deployment providers `--provider` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// GitHub Deployments, through `gh api`.
    Github,
}

/// Why a watch could not read its target (exit 7).
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// A `gh` call failed.
    #[error(transparent)]
    Gh(#[from] GhError),
    /// The review threads could be read neither by GraphQL nor from the Claude Code proxy's
    /// route; the GraphQL failure leads, the route's follows as the source.
    #[error("{graphql}; the review thread route of Claude Code's proxy failed too")]
    Threads {
        /// Why the GraphQL query failed.
        graphql: GhError,
        /// Why the proxy's route failed.
        #[source]
        proxy: GhError,
    },
    /// An answer was not the shape expected.
    #[error("gh did not return {what} as expected")]
    Json {
        /// What was asked for.
        what: &'static str,
        /// The decoding failure.
        #[source]
        source: serde_json::Error,
    },
    /// The pull request was closed without being merged: nothing is left to wait for.
    #[error("pull request {pr} is closed without being merged")]
    Closed {
        /// The pull request number.
        pr: u64,
    },
    /// GitHub has no such target.
    #[error("{what} not found")]
    Missing {
        /// The target, as `pull request 7` or `commit main`.
        what: String,
    },
    /// A ref that is empty, `.` or `..`: as a path segment it would name another endpoint, and
    /// git allows none of them as a ref.
    #[error("{git_ref:?} is not a branch, tag or commit")]
    BadRef {
        /// The ref as given.
        git_ref: String,
    },
    /// More items than are read, so the target cannot be judged whole.
    #[error("more {what} than the {limit} anneal reads")]
    TooMany {
        /// What there are too many of.
        what: &'static str,
        /// The most read.
        limit: usize,
    },
}

impl WatchError {
    /// The exit code: 7 for every query error. Exhaustive, so a new variant has to choose.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Gh(_)
            | Self::Threads { .. }
            | Self::Json { .. }
            | Self::Closed { .. }
            | Self::Missing { .. }
            | Self::BadRef { .. }
            | Self::TooMany { .. } => exit_codes::WATCH_QUERY,
        }
    }
}

/// The ports a watch runs with.
pub(crate) struct Ports<'a, G, H, C> {
    /// `gh`, for REST and GraphQL.
    pub(crate) gh: &'a G,
    /// The merge gate.
    pub(crate) gate: &'a H,
    /// Time.
    pub(crate) clock: &'a C,
}

/// Runs `anneal watch <action>` from `cwd` with the real `gh`, merge gate and clock, and
/// writes its envelope to `out`.
///
/// # Errors
/// [`CliError::Watch`] when the target cannot be read (exit 7), a usage error for a deployment
/// id the provider cannot take, or the failure to write `out`.
pub(crate) fn run<O: Write>(
    args: &WatchArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let ports = Ports {
        gh: &GhCli::new(),
        gate: &LedgerGate { cwd, lookup },
        clock: &SystemClock::start(),
    };
    execute(&args.action, &ports, out)
}

/// Runs `action` with `ports` and writes its envelope to `out`.
fn execute<G, H, C, O>(
    action: &WatchAction,
    ports: &Ports<'_, G, H, C>,
    out: &mut O,
) -> Result<Code, CliError>
where
    G: GhApi + GhGraphql,
    H: HeadGate,
    C: Clock,
    O: Write,
{
    let clock = ports.clock;
    let (target, wait, watched) = match action {
        WatchAction::Pr(args) => {
            let source = PullRequestWatch {
                gh: ports.gh,
                gate: ports.gate,
                pr: args.pr,
            };
            let watched = poll::watch(&source, clock, args.wait.schedule())?;
            (Target::Pr { number: args.pr }, args.wait, watched)
        }
        WatchAction::Ci(args) => {
            let repo = repo_ref(args.repo.as_ref());
            let source = CommitWatch {
                gh: ports.gh,
                repo,
                git_ref: &args.git_ref,
            };
            let watched = poll::watch(&source, clock, args.wait.schedule())?;
            let target = Target::Ci {
                git_ref: &args.git_ref,
                repo: repo.named(),
            };
            (target, args.wait, watched)
        }
        WatchAction::Deploy(args) => {
            let repo = repo_ref(args.repo.as_ref());
            let provider = GitHubDeployments {
                gh: ports.gh,
                repo,
                id: github_deployment_id(args)?,
            };
            let source = DeployWatch {
                provider: &provider,
                id: &args.id,
            };
            let watched = poll::watch(&source, clock, args.wait.schedule())?;
            let target = Target::Deploy {
                id: &args.id,
                provider: args.provider,
                repo: repo.named(),
            };
            (target, args.wait, watched)
        }
    };
    report(&watched, target, wait, out)
}

/// The repository `--repo` names, or gh's current one.
fn repo_ref(repo: Option<&Repo>) -> RepoRef<'_> {
    repo.map_or(RepoRef::Current, RepoRef::Named)
}

/// The GitHub deployment `args` names; a usage error unless its id is a number.
fn github_deployment_id(args: &DeployArgs) -> Result<u64, CliError> {
    let Provider::Github = args.provider;
    args.id
        .parse::<u64>()
        .map_err(|_not_a_number| CliError::Usage {
            reason: format!(
                "a GitHub deployment id is a number (gh api repos/{{owner}}/{{repo}}/deployments \
                 lists them), got {:?}",
                args.id
            ),
        })
}

/// Writes the envelope of `watched`, a watch of `target` that waited at most `wait`.
fn report<O: Write>(
    watched: &Watched,
    target: Target<'_>,
    wait: Wait,
    out: &mut O,
) -> Result<Code, CliError> {
    let (state, code) = outcome(watched);
    let snapshot = &watched.snapshot;
    let mut findings: Vec<Finding> = snapshot.blockers.iter().map(Blocker::finding).collect();
    if watched.timed_out {
        findings.extend(snapshot.pending.iter().map(Pending::finding));
    }
    snapshot::unique_keys(&mut findings);
    let subcommand = target.subcommand();
    let data = WatchData {
        target,
        state,
        head: snapshot.head.as_deref(),
        blockers: &snapshot.blockers,
        pending: &snapshot.pending,
        polls: watched.polls,
        waited_secs: watched.waited.as_secs(),
        timeout_secs: wait.timeout,
    };
    Envelope::new(CommandName::Watch, Some(subcommand), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// The state and exit code of a finished watch: the lowest blocker's code, else 5 on a
/// timeout, else 0.
fn outcome(watched: &Watched) -> (State, Code) {
    match watched.snapshot.blocker_code() {
        Some(code) => (State::Blocked, code),
        None if watched.timed_out => (State::Timeout, exit_codes::WATCH_TIMEOUT),
        None => (State::Clear, exit_codes::WATCH_GREEN),
    }
}

/// `data` of `anneal watch`.
#[derive(Debug, Serialize)]
struct WatchData<'a> {
    target: Target<'a>,
    state: State,
    /// The commit the checks were read for.
    head: Option<&'a str>,
    blockers: &'a [Blocker],
    /// What was still unfinished at the last poll.
    pending: &'a [Pending],
    polls: u32,
    waited_secs: u64,
    timeout_secs: u64,
}

/// What was watched.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Target<'a> {
    Pr {
        number: u64,
    },
    Ci {
        #[serde(rename = "ref")]
        git_ref: &'a str,
        repo: Option<String>,
    },
    Deploy {
        id: &'a str,
        provider: Provider,
        repo: Option<String>,
    },
}

impl Target<'_> {
    /// The subcommand that watched it.
    const fn subcommand(&self) -> &'static str {
        match self {
            Self::Pr { .. } => "pr",
            Self::Ci { .. } => "ci",
            Self::Deploy { .. } => "deploy",
        }
    }
}

/// How the watch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    /// Nothing blocks and nothing is pending.
    Clear,
    /// At least one blocker.
    Blocked,
    /// Work was still pending at the deadline.
    Timeout,
}

#[cfg(test)]
mod tests;
