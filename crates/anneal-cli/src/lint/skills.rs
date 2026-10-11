//! `anneal lint skills`: checks skill directories against the 21 skill design rules
//! (`docs/skill-rules.md`).
//!
//! Per-skill checks (`checks_spec`, `checks_body`, `checks_prose`) run once per skill and are
//! cached by content hash; set-level checks (`checks_set`) run every time. A `SKILL.md` that
//! cannot be read as UTF-8 frontmatter plus body gets one finding and no further checks.

pub mod checker;

mod cache;
mod checks_body;
mod checks_prose;
mod checks_set;
mod checks_spec;
pub(crate) mod discover;
mod document;
pub(crate) mod frontmatter;
pub(crate) mod listing;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Deserialize;

use crate::baseline::BaselineError;
use crate::exit_codes::{self, Code};
use crate::lint::finding::{LintFinding, Location};
use cache::{Cache, content_hash};
use checker::Checker;
use discover::{MAX_READ_BYTES, SkillDir, Surroundings, discover, read_capped};
use document::Skill;

pub(crate) use checks_set::vocabulary;

/// Arguments of `anneal lint skills`.
#[derive(Debug, Clone, Args)]
pub struct SkillsArgs {
    /// Skill directories, or directories of skills (each subdirectory holding a SKILL.md)
    #[arg(value_name = "PATH", default_value = "plugin/skills")]
    pub paths: Vec<PathBuf>,
    /// The session-start bootstrap index to check against its 200-word budget (rule 4)
    #[arg(long, value_name = "FILE")]
    pub index: Option<PathBuf>,
    /// hooks.metadata.json whose hook ids count as enforcers (rule 10); defaults to
    /// ../hooks/hooks.metadata.json relative to the first PATH, when that file exists
    #[arg(long, value_name = "FILE")]
    pub hooks: Option<PathBuf>,
    /// Content-hash snapshot file: unchanged skills replay their stored findings instead of
    /// being checked again
    #[arg(long, value_name = "FILE")]
    pub cache: Option<PathBuf>,
}

/// What a run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Every finding, sorted by file, line and checker.
    pub findings: Vec<LintFinding>,
    /// How many skill directories were linted.
    pub skills: usize,
    /// How many of them replayed cached per-skill findings.
    pub cached: usize,
}

/// Why `lint skills` or `lint terms` could not run.
#[derive(Debug, thiserror::Error)]
pub enum LintError {
    /// A path given on the command line does not exist.
    #[error("no such path: {path}", path = path.display())]
    MissingPath {
        /// The path as given.
        path: PathBuf,
    },
    /// A path given on the command line is the wrong kind (a file for a directory, or the
    /// reverse).
    #[error("{path} is not {expected}", path = path.display())]
    WrongKind {
        /// The path as given.
        path: PathBuf,
        /// What it should be.
        expected: &'static str,
    },
    /// A file given on the command line that must be read whole (the `lint terms` glossary)
    /// is larger than the read cap.
    #[error("{path} is {size} bytes, over the {MAX_READ_BYTES}-byte read cap", path = path.display())]
    TooLarge {
        /// The path as given.
        path: PathBuf,
        /// Its size in bytes.
        size: u64,
    },
    /// A file or directory could not be read.
    #[error("reading {path}", path = path.display())]
    Io {
        /// The path being read.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// The content-hash snapshot could not be read or written.
    #[error("lint snapshot")]
    Cache(#[from] BaselineError),
}

impl LintError {
    pub(crate) fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// The documented exit code: a missing path is a usage error, the rest internal.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::MissingPath { .. } | Self::WrongKind { .. } | Self::TooLarge { .. } => {
                exit_codes::USAGE
            }
            Self::Io { .. } | Self::Cache(_) => exit_codes::INTERNAL,
        }
    }
}

/// Lints the skills under `args.paths`.
///
/// # Errors
/// [`LintError`] when a path is missing, a file cannot be read, or the snapshot cannot be read
/// or written. Rule violations are findings, never errors.
pub fn lint(args: &SkillsArgs) -> Result<Outcome, LintError> {
    let dirs = discover(&args.paths)?;
    let (hook_ids, mut findings) = hook_ids(args)?;
    let loaded = dirs
        .iter()
        .map(Loaded::read)
        .collect::<Result<Vec<_>, _>>()?;
    let mut skills = Vec::with_capacity(loaded.len());
    for file in &loaded {
        match file.parse() {
            Ok(skill) => skills.push(skill),
            Err(finding) => findings.push(finding),
        }
    }
    let mut cache = args.cache.as_deref().map(Cache::load).transpose()?;
    let mut cached = 0;
    for skill in &skills {
        let (skill_findings, hit) = per_skill(skill, &hook_ids, cache.as_mut())?;
        findings.extend(skill_findings);
        cached += usize::from(hit);
    }
    if !dirs.is_empty() {
        findings.extend(set_level(args, &dirs, &skills)?);
    }
    if let (Some(cache), Some(path)) = (cache.as_mut(), args.cache.as_deref()) {
        cache.retain(&dirs.iter().map(|d| d.dir.display().to_string()).collect());
        cache.save(path)?;
    }
    findings.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    Ok(Outcome {
        findings,
        skills: dirs.len(),
        cached,
    })
}

/// The per-skill findings, replayed from `cache` when the content hash matches. Returns
/// whether it was a cache hit.
fn per_skill(
    skill: &Skill<'_>,
    hook_ids: &BTreeSet<String>,
    cache: Option<&mut Cache>,
) -> Result<(Vec<LintFinding>, bool), LintError> {
    let around = Surroundings::read(skill.dir)?;
    let Some(cache) = cache else {
        return Ok((check_skill(skill, &around, hook_ids), false));
    };
    let key = skill.dir.dir.display().to_string();
    let hash = content_hash(skill, &around, hook_ids);
    if let Some(stored) = cache.lookup(&key, &hash) {
        return Ok((stored.to_vec(), true));
    }
    let findings = check_skill(skill, &around, hook_ids);
    cache.store(key, hash, findings.clone());
    Ok((findings, false))
}

/// The per-skill checks. They read only `skill`, `around` and `hook_ids`, which is exactly what
/// [`content_hash`] covers.
fn check_skill(
    skill: &Skill<'_>,
    around: &Surroundings,
    hook_ids: &BTreeSet<String>,
) -> Vec<LintFinding> {
    let mut findings = checks_spec::check(skill);
    findings.extend(checks_body::check(skill, &around.references));
    findings.extend(checks_prose::check(skill, hook_ids, around.has_scripts));
    findings
}

fn set_level(
    args: &SkillsArgs,
    dirs: &[SkillDir],
    skills: &[Skill<'_>],
) -> Result<Vec<LintFinding>, LintError> {
    let names: BTreeSet<&str> = dirs.iter().map(|d| d.name.as_str()).collect();
    let root = args
        .paths
        .first()
        .map_or_else(String::new, |p| p.display().to_string());
    let mut findings: Vec<LintFinding> = checks_set::listing_budget(skills).into_iter().collect();
    findings.extend(checks_set::siblings(skills, &names));
    findings.extend(checks_set::overlap(skills));
    let index = match &args.index {
        Some(path) => Some((path.display().to_string(), read_text(path)?)),
        None => None,
    };
    findings.extend(match &index {
        None => checks_set::bootstrap_index(None, &root),
        Some((path, Ok(text))) => checks_set::bootstrap_index(Some((path, text)), &root),
        Some((path, Err(size))) => Some(LintFinding::error(
            Checker::BootstrapIndex,
            Location::new(path.as_str(), 1),
            format!("the bootstrap index is {size} bytes, far over its 200-word budget."),
            "keep only what anneal is, the entry skills with triggers, and the user-invoked \
             commands.",
        )),
    });
    findings.extend(checks_set::outside_skills(&root));
    Ok(findings)
}

/// A file named on the command line must exist and must not be a directory; either mistake is
/// a usage error.
pub(crate) fn require_file(path: &Path) -> Result<(), LintError> {
    if !path.exists() {
        return Err(LintError::MissingPath {
            path: path.to_path_buf(),
        });
    }
    if path.is_dir() {
        return Err(LintError::WrongKind {
            path: path.to_path_buf(),
            expected: "a file",
        });
    }
    Ok(())
}

/// Reads a text file given on the command line: its text, or `Err(size)` when it is larger
/// than the read cap. A missing file is a usage error. Invalid UTF-8 is replaced, which only
/// matters for word counts and JSON parse errors.
fn read_text(path: &Path) -> Result<Result<String, u64>, LintError> {
    require_file(path)?;
    let capped = read_capped(path)?;
    Ok(capped
        .bytes
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .ok_or(capped.size))
}

/// `hooks.metadata.json`: only the hook ids matter here.
#[derive(Debug, Deserialize)]
struct HooksMetadata {
    hooks: Vec<HookEntry>,
}

#[derive(Debug, Deserialize)]
struct HookEntry {
    id: String,
}

/// The hook ids that count as enforcers, plus a finding when the metadata does not parse.
fn hook_ids(args: &SkillsArgs) -> Result<(BTreeSet<String>, Vec<LintFinding>), LintError> {
    let path: Cow<'_, Path> = match (&args.hooks, args.paths.first()) {
        (Some(path), _) => Cow::Borrowed(path),
        (None, Some(first)) => {
            let default = first.join("../hooks/hooks.metadata.json");
            if !default.is_file() {
                return Ok((BTreeSet::new(), Vec::new()));
            }
            Cow::Owned(default)
        }
        (None, None) => return Ok((BTreeSet::new(), Vec::new())),
    };
    let parsed = read_text(&path)?
        .map_err(|size| format!("the file is {size} bytes"))
        .and_then(|text| {
            serde_json::from_str::<HooksMetadata>(&text).map_err(|error| error.to_string())
        });
    match parsed {
        Ok(metadata) => Ok((
            metadata.hooks.into_iter().map(|h| h.id).collect(),
            Vec::new(),
        )),
        Err(reason) => Ok((
            BTreeSet::new(),
            vec![LintFinding::error(
                Checker::Enforcer,
                Location::new(path.display().to_string(), 1),
                format!("hooks metadata does not parse: {reason}."),
                "fix the JSON: `{\"version\": 1, \"hooks\": [{\"id\": \"<hook-id>\", ...}]}`.",
            )],
        )),
    }
}

/// A `SKILL.md` as read from disk.
#[derive(Debug)]
struct Loaded<'d> {
    dir: &'d SkillDir,
    file: String,
    size: u64,
    bytes: Option<Vec<u8>>,
}

impl<'d> Loaded<'d> {
    fn read(dir: &'d SkillDir) -> Result<Self, LintError> {
        let path = dir.skill_file();
        let capped = read_capped(&path)?;
        Ok(Self {
            dir,
            file: path.display().to_string(),
            size: capped.size,
            bytes: capped.bytes,
        })
    }

    /// The parsed skill, or the one finding that stops it from being checked further.
    fn parse(&self) -> Result<Skill<'_>, LintFinding> {
        let at = Location::new(self.file.as_str(), 1);
        let Some(bytes) = &self.bytes else {
            return Err(checks_body::oversized(at, &format!("{} bytes", self.size)));
        };
        let unparseable = |line, reason: &str| {
            LintFinding::error(
                Checker::Frontmatter,
                Location::new(self.file.as_str(), line),
                format!("frontmatter does not parse: {reason}."),
                "write `---`, then `key: value` lines (or `>`/`|` blocks), then `---`.",
            )
        };
        let text = std::str::from_utf8(bytes).map_err(|error| {
            let reason = format!("SKILL.md is not UTF-8 after byte {}", error.valid_up_to());
            unparseable(1, &reason)
        })?;
        let doc = frontmatter::parse(text).map_err(|e| unparseable(e.line, e.reason))?;
        Ok(Skill::new(self.dir, &self.file, self.size, doc))
    }
}
