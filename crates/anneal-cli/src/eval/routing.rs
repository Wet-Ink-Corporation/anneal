//! `anneal eval routing`: Tier 1 lexical routing (`evals/README.md`).
//!
//! Each model-invoked skill has a fixture `<fixtures>/<skill>.yaml` of positive prompts, which
//! must rank the skill first, and near misses, which look like the skill but belong to another
//! (their owner), who must outscore it. Prompts are scored against every model-invoked skill
//! by word overlap (the private `score` module). The bar is 100%: one miss fails the run.

mod confusion;
mod fixture;
mod judge;
pub mod real;
mod score;

use std::ffi::OsStr;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use clap::Args;

pub use confusion::Confusion;

use crate::exit_codes::{self, Code};
use crate::lint::finding::LintFinding;
use crate::lint::skills::LintError;
use crate::lint::skills::discover::{EntryKind, list_dir, read_capped};
use crate::lint::skills::listing::listings;
use fixture::Fixture;
use judge::judge;

/// Arguments of `anneal eval routing`.
#[derive(Debug, Clone, Args)]
pub struct RoutingArgs {
    /// The directory of skills to route between
    #[arg(long, value_name = "DIR", default_value = "plugin/skills")]
    pub skills: PathBuf,
    /// The directory of routing fixtures, one `<skill>.yaml` per model-invoked skill
    #[arg(long, value_name = "DIR", default_value = "evals/routing")]
    pub fixtures: PathBuf,
}

/// The fewest prompts of each kind a fixture holds.
pub const MIN_PROMPTS: usize = 8;
/// The most prompts of each kind a fixture holds.
pub const MAX_PROMPTS: usize = 10;

/// The skill a prompt ranks first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Top {
    /// One skill scores strictly higher than every other.
    Skill(String),
    /// Several skills share the best score.
    Tie,
    /// No skill shares a content word with the prompt.
    NoMatch,
}

/// The label of [`Top::Tie`].
const TIE: &str = "(tie)";
/// The label of [`Top::NoMatch`].
const NO_MATCH: &str = "(none)";

impl Top {
    /// The column label: the skill's name, `(tie)` or `(none)`.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Skill(name) => name,
            Self::Tie => TIE,
            Self::NoMatch => NO_MATCH,
        }
    }
}

/// What a case checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseKind {
    /// The prompt must rank its skill first.
    Positive,
    /// The prompt's owner must outscore `mimics`, the skill whose fixture lists it.
    NearMiss {
        /// The skill the prompt resembles.
        mimics: String,
    },
}

/// One scored prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    /// The fixture file, for display.
    pub file: String,
    /// 1-based line of the prompt.
    pub line: usize,
    /// Positive or near miss.
    pub kind: CaseKind,
    /// The prompt.
    pub prompt: String,
    /// The skill that should win: the fixture's skill, or a near miss's owner.
    pub expected: String,
    /// The skill the prompt ranked first.
    pub top: Top,
    /// The expected skill's score.
    pub expected_score: usize,
    /// The skill it is measured against: the best other skill for a positive, the mimicked
    /// skill for a near miss. `None` when there is no other skill.
    pub rival: Option<Rival>,
    /// Whether the case meets the bar.
    pub passed: bool,
}

/// The skill a case's expected skill is measured against, and its score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rival {
    /// The rival skill's name.
    pub skill: String,
    /// Its score for the prompt.
    pub score: usize,
}

/// Which list of a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    /// `positives:`
    Positives,
    /// `near_misses:`
    NearMisses,
}

impl List {
    /// The key as written in a fixture.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Positives => "positives",
            Self::NearMisses => "near_misses",
        }
    }
}

/// What is wrong with a fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The file is larger than the read cap.
    TooLarge {
        /// Its size in bytes.
        size: u64,
    },
    /// The file is not UTF-8.
    NotUtf8,
    /// The file leaves the supported YAML subset.
    Parse {
        /// Why, one line.
        reason: &'static str,
    },
    /// The file name is not UTF-8, so it cannot name a skill.
    NameNotUtf8,
    /// No skill directory has the fixture's name.
    UnknownSkill,
    /// The skill sets `disable-model-invocation: true`, so nothing routes to it.
    NotRoutable,
    /// A list holds fewer than [`MIN_PROMPTS`] or more than [`MAX_PROMPTS`] prompts.
    Count {
        /// Which list.
        list: List,
        /// How many it holds.
        count: usize,
    },
    /// A near miss names an owner that is not a model-invoked skill.
    UnknownOwner {
        /// The owner as written.
        owner: String,
    },
    /// A near miss is owned by the skill whose fixture lists it.
    SelfOwned,
    /// The fixture's skill exists but its `SKILL.md` does not parse.
    SkillUnparseable,
    /// A near miss names an owner whose `SKILL.md` does not parse.
    OwnerUnparseable {
        /// The owner as written.
        owner: String,
    },
}

impl Display for Fault {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size } => write!(f, "the fixture is {size} bytes, over the read cap"),
            Self::NotUtf8 => f.write_str("the fixture is not UTF-8"),
            Self::Parse { reason } => write!(f, "the fixture does not parse: {reason}"),
            Self::NameNotUtf8 => f.write_str("the fixture's file name is not UTF-8"),
            Self::UnknownSkill => f.write_str("no skill has this fixture's name"),
            Self::NotRoutable => f.write_str(
                "the skill is user-invoked (disable-model-invocation); remove its fixture",
            ),
            Self::Count { list, count } => write!(
                f,
                "`{}` holds {count} prompts; it needs {MIN_PROMPTS} to {MAX_PROMPTS}",
                list.key()
            ),
            Self::UnknownOwner { owner } => {
                write!(f, "near-miss owner `{owner}` is not a model-invoked skill")
            }
            Self::SelfOwned => f.write_str("a near miss cannot be owned by the skill it mimics"),
            Self::SkillUnparseable => f.write_str(
                "the skill's SKILL.md does not parse; fix it first (see `anneal lint skills`)",
            ),
            Self::OwnerUnparseable { owner } => write!(
                f,
                "near-miss owner `{owner}`: its SKILL.md does not parse; fix it first (see `anneal lint skills`)"
            ),
        }
    }
}

/// Something that makes the run's result untrustworthy: it exits [`exit_codes::EVAL_INVALID`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A `SKILL.md` could not be read; the `lint skills` finding says why.
    Skill(LintFinding),
    /// A model-invoked skill has no fixture.
    MissingFixture {
        /// The skill.
        skill: String,
    },
    /// A fixture is wrong.
    Fixture {
        /// The fixture file, for display.
        file: String,
        /// 1-based line, or 0 for the file as a whole.
        line: usize,
        /// What is wrong.
        fault: Fault,
    },
}

/// What a routing run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The model-invoked skills, sorted.
    pub skills: Vec<String>,
    /// Every scored prompt, by fixture then line.
    pub cases: Vec<Case>,
    /// Every problem, by skill then fixture, then missing fixtures.
    pub problems: Vec<Problem>,
}

impl Outcome {
    /// The cases that miss the bar.
    pub fn misses(&self) -> impl Iterator<Item = &Case> + '_ {
        self.cases.iter().filter(|case| !case.passed)
    }

    /// The confusion table: expected skill against top-ranked skill.
    #[must_use]
    pub fn confusion(&self) -> Confusion<'_> {
        Confusion::new(&self.skills, &self.cases)
    }

    /// [`exit_codes::EVAL_INVALID`] on any problem, else [`exit_codes::EVAL_BELOW_BOUND`] on
    /// any miss, else [`exit_codes::OK`].
    #[must_use]
    pub fn code(&self) -> Code {
        if !self.problems.is_empty() {
            exit_codes::EVAL_INVALID
        } else if self.misses().next().is_some() {
            exit_codes::EVAL_BELOW_BOUND
        } else {
            exit_codes::OK
        }
    }
}

/// Why `eval routing` could not run.
#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    /// The skills directory could not be read.
    #[error("reading skills")]
    Skills(#[source] LintError),
    /// The fixtures directory could not be read.
    #[error("reading fixtures")]
    Fixtures(#[source] LintError),
    /// The real router (`--router claude`) could not run.
    #[error("the real router")]
    Router(#[source] real::RouterError),
    /// `--changed-since` could not compare the skills with its base.
    #[error("--changed-since")]
    Changed(#[source] real::ChangedError),
}

impl RoutingError {
    /// The documented exit code: a missing or wrong-kind path is a usage error, the rest
    /// internal; the real router's failures are [`real::RouterError::code`]'s.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Skills(error) | Self::Fixtures(error) => error.code(),
            Self::Router(error) => error.code(),
            Self::Changed(error) => error.code(),
        }
    }
}

/// Scores every fixture under `args.fixtures` against the skills under `args.skills`.
///
/// # Errors
/// [`RoutingError`] when a directory is missing, the wrong kind, or cannot be read. Bad
/// fixtures and unreadable `SKILL.md` files are [`Problem`]s, not errors.
pub fn evaluate(args: &RoutingArgs) -> Result<Outcome, RoutingError> {
    let skills = listings(std::slice::from_ref(&args.skills)).map_err(RoutingError::Skills)?;
    let sources = read_fixtures(&args.fixtures).map_err(RoutingError::Fixtures)?;
    Ok(judge(skills, sources))
}

/// A fixture file as read from disk.
#[derive(Debug)]
struct Source {
    /// The path, for display.
    file: String,
    /// The file stem, or `None` when it is not UTF-8.
    skill: Option<String>,
    /// The parsed fixture, or the line and fault that stopped it.
    parsed: Result<Fixture, (usize, Fault)>,
}

/// The `*.yaml` files in `dir`, sorted by name; other entries are ignored.
fn read_fixtures(dir: &Path) -> Result<Vec<Source>, LintError> {
    if !dir.exists() {
        return Err(LintError::MissingPath {
            path: dir.to_path_buf(),
        });
    }
    if !dir.is_dir() {
        return Err(LintError::WrongKind {
            path: dir.to_path_buf(),
            expected: "a directory of routing fixtures",
        });
    }
    let mut sources = Vec::new();
    for (name, kind) in list_dir(dir)? {
        let path = dir.join(&name);
        let is_yaml = path.extension() == Some(OsStr::new("yaml"));
        let is_file = match kind {
            EntryKind::File => true,
            EntryKind::Symlink => path.is_file(),
            EntryKind::Dir | EntryKind::Other => false,
        };
        if !is_yaml || !is_file {
            continue;
        }
        let capped = read_capped(&path)?;
        let parsed = match capped.bytes {
            None => Err((0, Fault::TooLarge { size: capped.size })),
            Some(bytes) => String::from_utf8(bytes)
                .map_err(|_not_utf8| (0, Fault::NotUtf8))
                .and_then(|text| {
                    fixture::parse(&text).map_err(|e| (e.line, Fault::Parse { reason: e.reason }))
                }),
        };
        sources.push(Source {
            file: path.display().to_string(),
            skill: path.file_stem().and_then(OsStr::to_str).map(str::to_owned),
            parsed,
        });
    }
    Ok(sources)
}

#[cfg(test)]
mod read_tests;
#[cfg(test)]
mod tests;
