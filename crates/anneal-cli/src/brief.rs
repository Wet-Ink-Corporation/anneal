//! `anneal brief build`: the brief a subagent is spawned with, and the review package (diff and
//! contract) its reviewer reads, both written as files under `.anneal/briefs/`.
//!
//! - `fields`: the nine fields, in one table.
//! - `ticket`: the validated ticket name the files are named after.
//! - `render`: the text of the brief and of the review package.

pub mod fields;
mod render;
pub mod ticket;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anneal_ledger::Timestamp;
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::baseline::write_atomically;
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::lint::brief::{Rule, check};
use crate::paths::{self, GIT_TIMEOUT, PathError};
use crate::pr_diff::{self, DiffError, TextLimits};
use render::{Given, Range};
use ticket::Ticket;

/// Most bytes of diff a review package holds; a longer diff is named by its command instead.
pub const MAX_DIFF_BYTES: u64 = 4 * 1024 * 1024;

/// `anneal brief`.
#[derive(Debug, Args)]
pub struct BriefArgs {
    #[command(subcommand)]
    action: BriefAction,
}

/// The `anneal brief` subcommands.
#[derive(Debug, Subcommand)]
pub enum BriefAction {
    /// Write a ticket's brief and review package from the range BASE...HEAD
    Build(BuildArgs),
}

/// `anneal brief build`.
#[derive(Debug, Args)]
pub struct BuildArgs {
    /// The ticket: an issue number or a dag ticket id; it names the files
    #[arg(long, value_name = "ID")]
    ticket: String,
    /// The base the change starts from; it must be an ancestor of HEAD
    #[arg(long, value_name = "REF")]
    base: String,
    /// A field's text, as key=text (goal, repository, owned-scope, do-not-touch, invariants,
    /// success-criteria, verification, edits-allowed, return-contract); repeatable
    #[arg(long = "field", value_name = "KEY=TEXT")]
    fields: Vec<String>,
    /// The subagent waits on CI: write `cacheTtl: 1h` in the volatile section
    #[arg(long)]
    waits_on_ci: bool,
}

/// Why `brief build` could not write its files.
#[derive(Debug, thiserror::Error)]
pub enum BriefError {
    /// The working directory is not in a git worktree.
    #[error("anneal brief build needs a git worktree")]
    NotAWorktree(#[source] PathError),
    /// git could not resolve the range or read the diff.
    #[error("reading the range")]
    Git(#[from] DiffError),
    /// A file could not be written.
    #[error("writing {}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
}

impl BriefError {
    /// The documented exit code: a usage error (2) outside a worktree, else internal (70).
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::NotAWorktree(_) => exit_codes::USAGE,
            Self::Git(_) | Self::Write { .. } => exit_codes::INTERNAL,
        }
    }
}

/// Runs `anneal brief <action>` in `cwd`.
pub(crate) fn run<O: Write>(args: &BriefArgs, cwd: &Path, out: &mut O) -> Result<Code, CliError> {
    match &args.action {
        BriefAction::Build(build) => run_build(build, cwd, out),
    }
}

/// The `--field` values, checked: a known key, given once, with text.
fn given(values: &[String]) -> Result<Vec<Given<'_>>, CliError> {
    let mut given: Vec<Given<'_>> = Vec::with_capacity(values.len());
    for value in values {
        let usage = |reason: String| CliError::Usage { reason };
        let (key, text) = value
            .split_once('=')
            .ok_or_else(|| usage(format!("--field {value:?}: write it as key=text")))?;
        if fields::by_key(key).is_none() {
            let keys: Vec<&str> = fields::FIELDS.iter().map(|field| field.key).collect();
            return Err(usage(format!(
                "--field {key:?}: not a brief field; one of {}",
                keys.join(", ")
            )));
        }
        if text.trim().is_empty() {
            return Err(usage(format!("--field {key}: the text is empty")));
        }
        if given.iter().any(|earlier| earlier.key == key) {
            return Err(usage(format!("--field {key}: given twice")));
        }
        given.push(Given { key, text });
    }
    Ok(given)
}

/// Why the range is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BadRange {
    /// The base names no commit.
    UnknownBase,
    /// `HEAD` names no commit (no commit yet).
    NoHead,
    /// The base is not an ancestor of `HEAD`, so the range is not rooted at it.
    NotAncestor,
}

impl BadRange {
    fn message(self, base: &str) -> String {
        match self {
            Self::UnknownBase => format!("--base {base}: names no commit in this repository"),
            Self::NoHead => "HEAD names no commit: commit first".to_owned(),
            Self::NotAncestor => format!(
                "--base {base} is not an ancestor of HEAD, so {base}...HEAD is not rooted at it; \
                 rebase onto it or pass the base the branch started from"
            ),
        }
    }
}

/// The base's and head's full commit ids, or why the range is refused.
fn resolve_range(root: &Path, base: &str) -> Result<Result<(String, String), BadRange>, DiffError> {
    let Some(base_commit) = pr_diff::resolve_commit(root, base, GIT_TIMEOUT)? else {
        return Ok(Err(BadRange::UnknownBase));
    };
    let Some(head) = pr_diff::resolve_commit(root, "HEAD", GIT_TIMEOUT)? else {
        return Ok(Err(BadRange::NoHead));
    };
    if pr_diff::is_ancestor(root, &base_commit, &head, GIT_TIMEOUT)? {
        Ok(Ok((base_commit, head)))
    } else {
        Ok(Err(BadRange::NotAncestor))
    }
}

/// `data` of `brief build`.
#[derive(Debug, Serialize)]
struct BuildData<'a> {
    ticket: &'a str,
    base: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_commit: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brief: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_package: Option<String>,
    /// Fields still to fill (`lint brief` fails until they are).
    empty_fields: Vec<&'static str>,
    diff_omitted: bool,
    waits_on_ci: bool,
}

/// `anneal brief build`.
fn run_build<O: Write>(args: &BuildArgs, cwd: &Path, out: &mut O) -> Result<Code, CliError> {
    let ticket = args.ticket.parse::<Ticket>()?;
    let given = given(&args.fields)?;
    let root = paths::worktree_root_within(cwd, GIT_TIMEOUT).map_err(|error| match error {
        PathError::NotARepository { .. } => CliError::Brief(BriefError::NotAWorktree(error)),
        other => CliError::Path(other),
    })?;
    let root = Path::new(&root);
    let (base_commit, head) = match resolve_range(root, &args.base).map_err(BriefError::from)? {
        Ok(range) => range,
        Err(bad) => return bad_range(&ticket, &args.base, bad, out),
    };
    let limits = TextLimits {
        timeout: GIT_TIMEOUT,
        max_bytes: MAX_DIFF_BYTES,
    };
    let diff = pr_diff::diff_text(root, &base_commit, &head, limits)
        .map_err(BriefError::from)?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let built = Timestamp::now()?;
    let repository = format!(
        "`{}` (git worktree); the change starts from `{}`.",
        root.display(),
        args.base
    );
    let inputs = render::Brief {
        ticket: &ticket,
        given: &given,
        repository: &repository,
        range: Range {
            base: &args.base,
            base_commit: &base_commit,
            head: &head,
        },
        waits_on_ci: args.waits_on_ci,
        built: built.as_str(),
    };
    let empty_fields = write_files(root, &inputs, diff.as_deref())?;
    let data = BuildData {
        ticket: ticket.as_str(),
        base: &args.base,
        base_commit: Some(&base_commit),
        head: Some(&head),
        brief: Some(brief_file(&ticket)),
        review_package: Some(package_file(&ticket)),
        empty_fields,
        diff_omitted: diff.is_none(),
        waits_on_ci: args.waits_on_ci,
    };
    Envelope::new(CommandName::Brief, Some("build"), exit_codes::OK, data)
        .emit(out)
        .map_err(CliError::Output)
}

/// Writes the brief and the review package of `inputs` under `root`; returns the fields the
/// brief still has empty.
fn write_files(
    root: &Path,
    inputs: &render::Brief<'_>,
    diff: Option<&str>,
) -> Result<Vec<&'static str>, BriefError> {
    let brief_text = render::brief(inputs);
    write_under(root, &brief_file(inputs.ticket), &brief_text)?;
    write_under(
        root,
        &package_file(inputs.ticket),
        &render::review_package(inputs, diff),
    )?;
    Ok(check(&brief_text)
        .into_iter()
        .filter(|finding| finding.rule == Rule::Empty)
        .map(|finding| finding.field)
        .collect())
}

/// Writes `text` to `relative` (`/`-separated) under `root`, creating its directory.
fn write_under(root: &Path, relative: &str, text: &str) -> Result<(), BriefError> {
    let path = relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part));
    let written = match path.parent() {
        Some(dir) => std::fs::create_dir_all(dir),
        None => Ok(()),
    }
    .and_then(|()| write_atomically(&path, text.as_bytes()));
    written.map_err(|source| BriefError::Write { path, source })
}

/// The envelope for a refused range: exit 2, one `brief/range` finding, nothing written.
fn bad_range<O: Write>(
    ticket: &Ticket,
    base: &str,
    bad: BadRange,
    out: &mut O,
) -> Result<Code, CliError> {
    let findings = [Finding::new("brief/range", bad.message(base))];
    let data = BuildData {
        ticket: ticket.as_str(),
        base,
        base_commit: None,
        head: None,
        brief: None,
        review_package: None,
        empty_fields: Vec::new(),
        diff_omitted: false,
        waits_on_ci: false,
    };
    Envelope::new(
        CommandName::Brief,
        Some("build"),
        exit_codes::BRIEF_BAD_RANGE,
        data,
    )
    .with_findings(&findings)
    .emit(out)
    .map_err(CliError::Output)
}

/// The directory briefs and review packages are written to, relative to the worktree root.
pub const BRIEFS_DIR: &str = ".anneal/briefs";

/// The brief of `ticket`, relative to the worktree root, `/`-separated.
#[must_use]
pub fn brief_file(ticket: &Ticket) -> String {
    format!("{BRIEFS_DIR}/{ticket}.md")
}

/// The review package of `ticket`, relative to the worktree root, `/`-separated.
#[must_use]
pub fn package_file(ticket: &Ticket) -> String {
    format!("{BRIEFS_DIR}/{ticket}.review.md")
}
