//! `anneal mode build|diagnose|off`: the anneal mode of a worktree, which the preconditions
//! hook (`hook::preconditions`) enforces.
//!
//! Each command appends one `mode_set` event whose envelope names the worktree (its key,
//! `scope::worktree_key` of the top-level directory git prints), so the latest `mode_set`
//! tagged with a worktree is its mode. `build` records the plan and design tree, relative to
//! the worktree root, and whether the working tree was clean (untracked files included); a
//! dirty tree is still recorded, with a finding, and the hook then denies source edits until
//! build mode is entered clean. See `docs/preconditions.md`.
//!
//! - `scope`: which paths the hook guards.

pub(crate) mod scope;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anneal_ledger::event::{Mode, ModeSet};
use anneal_ledger::{
    EventEnvelope, EventKind, Filter, LedgerEvent, LedgerTag, Recorded, Store, StoreError, TagKey,
    Timestamp,
};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths::{self, GIT_TIMEOUT, PathError, StoreLocation, StoreSource};

/// Most bytes of `git status --porcelain` kept: one is enough to know the tree is dirty; the
/// rest of the cap keeps git's error message readable.
const STATUS_CAP_BYTES: u64 = 4096;

/// `anneal mode`.
#[derive(Debug, Args)]
pub struct ModeArgs {
    #[command(subcommand)]
    action: ModeAction,
}

/// The `anneal mode` subcommands.
#[derive(Debug, Subcommand)]
pub enum ModeAction {
    /// Enter build mode: source edits need a spec that lints clean, the confirmed design tree,
    /// a clean tree at entry and the approved plan
    Build(BuildArgs),
    /// Enter diagnose mode: source edits need a red reproduction recorded after entry
    Diagnose(RecordArgs),
    /// Leave the anneal mode: the preconditions hook enforces nothing
    Off(RecordArgs),
}

/// `anneal mode build`.
#[derive(Debug, Args)]
pub struct BuildArgs {
    /// The approved plan (inside the worktree)
    #[arg(long, value_name = "FILE")]
    plan: PathBuf,
    /// The confirmed design tree (inside the worktree)
    #[arg(long, value_name = "FILE")]
    tree: PathBuf,
    #[command(flatten)]
    record: RecordArgs,
}

/// For whom `anneal mode` records. There is no `--store`: the mode goes to the ledger the
/// preconditions hook reads (`$ANNEAL_STORE`, else the git common dir, else the folder).
#[derive(Debug, Args)]
pub struct RecordArgs {
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
}

/// Why `anneal mode` could not record the mode.
#[derive(Debug, thiserror::Error)]
pub enum ModeError {
    /// The working directory is not in a git worktree.
    #[error("anneal mode needs a git worktree")]
    NotAWorktree(#[source] PathError),
    /// A `--plan` or `--tree` that does not name a regular file.
    #[error("{flag} {}: not a file", path.display())]
    NotAFile {
        /// The flag.
        flag: &'static str,
        /// The path as given.
        path: PathBuf,
    },
    /// A `--plan` or `--tree` outside the worktree, or one that is not UTF-8.
    #[error("{flag} {}: must be a UTF-8 path inside the worktree {root}", path.display())]
    Outside {
        /// The flag.
        flag: &'static str,
        /// The path as given.
        path: PathBuf,
        /// The worktree's top-level directory.
        root: String,
    },
    /// A path could not be resolved.
    #[error("resolving {}", path.display())]
    Resolve {
        /// The path.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

impl ModeError {
    /// The documented exit code: 70 when a path could not be resolved, else a usage error (2).
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::NotAWorktree(_) | Self::NotAFile { .. } | Self::Outside { .. } => {
                exit_codes::USAGE
            }
            Self::Resolve { .. } => exit_codes::INTERNAL,
        }
    }
}

/// Runs `anneal mode <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &ModeArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let (mode, subcommand, record) = match &args.action {
        ModeAction::Build(build) => (Mode::Build, "build", &build.record),
        ModeAction::Diagnose(record) => (Mode::Diagnose, "diagnose", record),
        ModeAction::Off(record) => (Mode::Off, "off", record),
    };
    let root = paths::worktree_root_within(cwd, GIT_TIMEOUT).map_err(|error| match error {
        PathError::NotARepository { .. } => CliError::Mode(ModeError::NotAWorktree(error)),
        other => CliError::Path(other),
    })?;
    let (plan, tree) = match &args.action {
        ModeAction::Build(build) => (
            Some(artifact(&root, cwd, "--plan", &build.plan)?),
            Some(artifact(&root, cwd, "--tree", &build.tree)?),
        ),
        ModeAction::Diagnose(_) | ModeAction::Off(_) => (None, None),
    };
    let clean = is_clean(Path::new(&root))?;
    let event = LedgerEvent::ModeSet(ModeSet {
        mode,
        clean,
        plan,
        tree,
    });
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.worktree = Some(scope::worktree_key(&root));
    envelope.session_id.clone_from(&record.session);
    // The worktree, plan and session become ledger tags; check them before a ledger is opened
    // (or created), so a value the store would refuse is a usage error that leaves no file.
    if let Err(error) = envelope.tags() {
        return Err(CliError::Usage {
            reason: format!(
                "the worktree, --plan and --session are recorded as ledger tags: {error}"
            ),
        });
    }
    let location = paths::ledger_store(None, lookup, cwd)?;
    let position = Store::open(&location.path)?.append(&envelope)?;
    let recorded = Recording {
        subcommand,
        envelope: &envelope,
        location: &location,
        position,
    };
    emit(&recorded, clean, out)
}

/// What `anneal mode` appended, and where.
struct Recording<'a> {
    subcommand: &'static str,
    envelope: &'a EventEnvelope,
    location: &'a StoreLocation,
    position: u64,
}

/// `data` of `anneal mode`: the event's payload (`type`, `mode`, `clean`, `plan`, `tree`),
/// its worktree and where it was recorded.
#[derive(Debug, Serialize)]
struct ModeData<'a> {
    #[serde(flatten)]
    event: &'a LedgerEvent,
    worktree: Option<&'a str>,
    store: &'a Path,
    source: StoreSource,
    position: u64,
}

fn emit<O: Write>(recorded: &Recording<'_>, clean: bool, out: &mut O) -> Result<Code, CliError> {
    let data = ModeData {
        event: &recorded.envelope.event,
        worktree: recorded.envelope.worktree.as_deref(),
        store: &recorded.location.path,
        source: recorded.location.source,
        position: recorded.position,
    };
    let findings = if clean {
        Vec::new()
    } else {
        vec![Finding::new(
            "mode/dirty",
            "the working tree has changes (untracked files included), so the preconditions \
             hook denies source edits in build mode. fix: commit or stash them, then run \
             `anneal mode build` again.",
        )]
    };
    let subcommand = Some(recorded.subcommand);
    Envelope::new(
        CommandName::Mode,
        subcommand,
        exit_codes::MODE_RECORDED,
        data,
    )
    .with_findings(&findings)
    .emit(out)
    .map_err(CliError::Output)
}

/// `path` (relative to `cwd`) relative to the worktree `root`, `/`-separated: it must be a
/// regular file inside the worktree.
fn artifact(root: &str, cwd: &Path, flag: &'static str, path: &Path) -> Result<String, ModeError> {
    // The errors own the path they name.
    let outside = || ModeError::Outside {
        flag,
        path: path.to_path_buf(),
        root: root.to_owned(),
    };
    let not_a_file = || ModeError::NotAFile {
        flag,
        path: path.to_path_buf(),
    };
    let full = cwd.join(path);
    if full.to_str().is_none() {
        return Err(outside());
    }
    // Any failure to stat it (missing, unreadable) means it is not a file anneal can use.
    if !fs::metadata(&full).is_ok_and(|metadata| metadata.is_file()) {
        return Err(not_a_file());
    }
    match worktree_relative(Path::new(root), &full) {
        Ok(Some(relative)) => Ok(relative),
        Ok(None) => Err(outside()),
        Err(source) => Err(ModeError::Resolve { path: full, source }),
    }
}

/// Whether the worktree at `root` has no change git reports: `git status --porcelain` with
/// untracked (not ignored) files, empty.
fn is_clean(root: &Path) -> Result<bool, PathError> {
    let output = paths::run_git_capped(
        root,
        &["status", "--porcelain", "--untracked-files=normal"],
        GIT_TIMEOUT,
        STATUS_CAP_BYTES,
    )?;
    if !output.status.success() {
        return Err(paths::git_failure(root, &output.stderr));
    }
    Ok(output.stdout.is_empty())
}

/// `path` (absolute) relative to the worktree `root`, `/`-separated, `None` outside it.
/// Symlinks are resolved in `root` and in the longest existing prefix of `path` (git prints
/// the root resolved, a tool may name the file through a symlinked directory such as macOS's
/// `/var`); the rest of `path` is taken lexically. Letters compare in any ASCII case.
///
/// # Errors
/// The I/O error when `root` or an existing prefix of `path` cannot be resolved.
pub(crate) fn worktree_relative(root: &Path, path: &Path) -> io::Result<Option<String>> {
    let root = scope::normalise(&fs::canonicalize(root)?.to_string_lossy());
    let path = scope::normalise(&resolved(path)?.to_string_lossy());
    Ok(scope::relative(&root, &path).map(str::to_owned))
}

/// `path` with symlinks resolved in its longest existing prefix, the rest appended as is.
fn resolved(path: &Path) -> io::Result<PathBuf> {
    let lexical = PathBuf::from(scope::normalise(&path.to_string_lossy()));
    let mut existing = lexical.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(mut found) => {
                found.extend(missing.iter().rev());
                return Ok(found);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
                    return Err(error);
                };
                missing.push(name);
                existing = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

/// The mode a worktree is in, from its latest `mode_set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Active {
    /// Build mode, with what it was entered with.
    Build(BuildMode),
    /// Build mode recorded without a plan or a design tree (appended by hand): nothing can be
    /// checked, so the hook denies source edits.
    IncompleteBuild,
    /// Diagnose mode.
    Diagnose,
}

/// What build mode was entered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildMode {
    /// The plan, relative to the worktree root.
    pub(crate) plan: String,
    /// The design tree, relative to the worktree root.
    pub(crate) tree: String,
    /// Whether the working tree was clean at entry.
    pub(crate) clean: bool,
}

/// A worktree's active mode and the ledger position it was entered at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entered {
    /// The `mode_set` event's position.
    pub(crate) position: u64,
    /// The mode.
    pub(crate) mode: Active,
}

/// The active mode of the worktree whose top-level directory git prints as `root`: the latest
/// `mode_set` tagged with its key ([`scope::worktree_key`]), or `None` when there is none or
/// it is `off`.
///
/// # Errors
/// The [`StoreError`] when the ledger cannot be read.
pub(crate) fn active(store: &Store, root: &str) -> Result<Option<Entered>, StoreError> {
    let tag = match LedgerTag::new(TagKey::Worktree, &scope::worktree_key(root)) {
        Ok(tag) => tag,
        // `anneal mode` refuses a worktree the ledger cannot tag, so none has a mode.
        Err(_untaggable) => return Ok(None),
    };
    let filter = Filter {
        types: vec![EventKind::ModeSet],
        tags: vec![tag],
    };
    Ok(store.tail(&filter, 1)?.pop().and_then(entered))
}

/// The mode `recorded` entered; `None` for `off` (or another event type, which the filter
/// excludes).
fn entered(recorded: Recorded) -> Option<Entered> {
    let LedgerEvent::ModeSet(set) = recorded.event.event else {
        return None;
    };
    let mode = match set {
        ModeSet {
            mode: Mode::Build,
            plan: Some(plan),
            tree: Some(tree),
            clean,
        } => Active::Build(BuildMode { plan, tree, clean }),
        ModeSet {
            mode: Mode::Build, ..
        } => Active::IncompleteBuild,
        ModeSet {
            mode: Mode::Diagnose,
            ..
        } => Active::Diagnose,
        ModeSet {
            mode: Mode::Off, ..
        } => return None,
    };
    Some(Entered {
        position: recorded.position,
        mode,
    })
}
