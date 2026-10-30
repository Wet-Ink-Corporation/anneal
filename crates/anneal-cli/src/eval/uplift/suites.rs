//! Reading `evals/uplift/<skill>/{regression,capability}/<case>.json`.
//!
//! Directories are listed without following symlinks (a symlinked skill, suite or case is a
//! problem); other files, such as a `README.md`, are ignored. `fixtures/` holds the cases'
//! fixtures, not a skill's suites. Every case file is read, so one bad case is reported beside
//! the others rather than hiding them; any problem stops a run before anything is spent.

use std::ffi::OsStr;
use std::path::Path;

use serde::Serialize;

use super::case::{self, Parsed};
use super::fixture::{self, FIXTURES_DIR, Fixture, RepoRoot};
use crate::lint::skills::LintError;
use crate::lint::skills::discover::{EntryKind, list_dir, read_bounded};

/// The most cases read in one run (H-13): past it the rest are a problem, not read.
const MAX_CASES: usize = 512;
/// The longest case name.
const MAX_SLUG: usize = 64;
/// The file extension of a case.
const CASE_EXTENSION: &str = ".json";
/// What separates a case name's skill, suite and slug ([`Case::name`]); a skill directory
/// may not hold it, so names stay unique.
const NAME_SEPARATOR: &str = "--";
/// The reason given for a symlink anywhere in the suites.
const SYMLINK: &str = "a symlink; suites and cases are read only as plain directories and files";

/// Which suite a case is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SuiteKind {
    /// Expected to pass with the skill nearly every time.
    Regression,
    /// Expected to pass rarely for now, and to rise; at 100% it graduates to regression.
    Capability,
}

impl SuiteKind {
    /// The suite's directory name.
    pub(crate) const fn dir(self) -> &'static str {
        match self {
            Self::Regression => "regression",
            Self::Capability => "capability",
        }
    }

    fn of(name: &str) -> Option<Self> {
        [Self::Regression, Self::Capability]
            .into_iter()
            .find(|kind| kind.dir() == name)
    }
}

/// A case placed in its skill and suite.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Case {
    /// The skill it measures.
    pub(crate) skill: String,
    /// Its suite.
    pub(crate) suite: SuiteKind,
    /// Its file stem.
    pub(crate) slug: String,
    /// Its file, relative to the suites directory, with `/`.
    pub(crate) file: String,
    /// What the file holds, its fixture name taken out into [`Case::fixture`], so
    /// `body.fixture` is always `None` here.
    pub(crate) body: Parsed,
    /// The fixture that seeds its runs, located ([`fixture::locate`]), when it names one.
    pub(crate) fixture: Option<Fixture>,
}

impl Case {
    /// Its name in a `claude plugin eval` run, unique across skills and suites:
    /// `<skill>--<suite>--<slug>`. A skill directory holding `--` is refused when read, so
    /// the first `--` always ends the skill and the suite word follows.
    pub(crate) fn name(&self) -> String {
        let sep = NAME_SEPARATOR;
        format!("{}{sep}{}{sep}{}", self.skill, self.suite.dir(), self.slug)
    }
}

/// Something wrong with the suites, by file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Problem {
    /// The file or directory, relative to the suites directory, with `/`.
    pub(crate) file: String,
    /// What is wrong, one line.
    pub(crate) reason: String,
}

/// How many valid cases a skill has in each suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Coverage {
    /// The skill.
    pub(crate) skill: String,
    /// Valid regression cases.
    pub(crate) regression: usize,
    /// Valid capability cases.
    pub(crate) capability: usize,
}

/// Everything read from the suites directory.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Suites {
    /// The valid cases, by skill, suite and name.
    pub(crate) cases: Vec<Case>,
    /// Every problem found.
    pub(crate) problems: Vec<Problem>,
    /// Each skill directory's valid cases per suite, by skill.
    pub(crate) coverage: Vec<Coverage>,
}

impl Suites {
    fn problem(&mut self, file: String, reason: impl Into<String>) {
        self.problems.push(Problem {
            file,
            reason: reason.into(),
        });
    }
}

/// The suites under `dir`, for the skills named in `skills`.
///
/// # Errors
/// [`LintError::MissingPath`] or [`LintError::WrongKind`] when `dir` is not a directory;
/// [`LintError::Io`] when a directory or file cannot be read.
pub(crate) fn read(dir: &Path, skills: &[String]) -> Result<Suites, LintError> {
    if !dir.exists() {
        return Err(LintError::MissingPath {
            path: dir.to_path_buf(),
        });
    }
    if !dir.is_dir() {
        return Err(LintError::WrongKind {
            path: dir.to_path_buf(),
            expected: "a directory",
        });
    }
    let mut suites = Suites::default();
    let source = Source {
        dir,
        root: RepoRoot::new(dir),
    };
    for (name, kind) in list_dir(dir)? {
        let shown = display(&name);
        match kind {
            EntryKind::Symlink => suites.problem(shown, SYMLINK),
            EntryKind::Dir if shown == FIXTURES_DIR => {
                if skills.contains(&shown) {
                    suites.problem(
                        shown,
                        "`fixtures` holds the case fixtures, so the skill `fixtures` cannot \
                         have suites",
                    );
                }
            }
            EntryKind::Dir if shown.contains(NAME_SEPARATOR) => {
                suites.problem(
                    shown,
                    "a skill directory with suites may not hold `--`, which separates the \
                     parts of a case's name",
                );
            }
            EntryKind::Dir if skills.contains(&shown) => {
                read_skill(&source, &shown, &mut suites)?;
            }
            EntryKind::Dir => {
                let reason = format!("no skill `{shown}` under the skills directory");
                suites.problem(shown, reason);
            }
            EntryKind::File | EntryKind::Other => {}
        }
    }
    // A suite's own problem is found after its files'; report in path order.
    suites.problems.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(suites)
}

/// The suites directory being read, and the repository its fixtures' symlinks must stay in.
struct Source<'a> {
    dir: &'a Path,
    root: RepoRoot<'a>,
}

/// The suites of `skill`, the directory `<suites>/<skill>`.
fn read_skill(source: &Source<'_>, skill: &str, suites: &mut Suites) -> Result<(), LintError> {
    let mut coverage = Coverage {
        skill: skill.to_owned(),
        regression: 0,
        capability: 0,
    };
    let mut has_suite = false;
    for (name, kind) in list_dir(&source.dir.join(skill))? {
        let shown = format!("{skill}/{}", display(&name));
        let suite = name.to_str().and_then(SuiteKind::of);
        // A symlinked suite is a problem of its own; it still names a suite.
        has_suite |= suite.is_some() && matches!(kind, EntryKind::Dir | EntryKind::Symlink);
        match (kind, suite) {
            (EntryKind::Symlink, _) => suites.problem(shown, SYMLINK),
            (EntryKind::Dir, Some(suite)) => {
                let read = read_suite(source, skill, suite, suites)?;
                match suite {
                    SuiteKind::Regression => coverage.regression = read,
                    SuiteKind::Capability => coverage.capability = read,
                }
            }
            (EntryKind::Dir, None) => {
                suites.problem(shown, "a suite is `regression` or `capability`");
            }
            (EntryKind::File | EntryKind::Other, _) => {}
        }
    }
    if !has_suite {
        suites.problem(skill.to_owned(), "no `regression` or `capability` suite");
    }
    suites.coverage.push(coverage);
    Ok(())
}

/// The cases of one suite; returns how many were valid. A case whose fixture cannot seed a
/// run is a problem, not a valid case.
fn read_suite(
    source: &Source<'_>,
    skill: &str,
    suite: SuiteKind,
    suites: &mut Suites,
) -> Result<usize, LintError> {
    let path = source.dir.join(skill).join(suite.dir());
    let place = format!("{skill}/{}", suite.dir());
    let mut files = 0_usize;
    let mut valid = 0_usize;
    for (name, kind) in list_dir(&path)? {
        let shown = display(&name);
        let Some(slug) = shown.strip_suffix(CASE_EXTENSION) else {
            continue;
        };
        let file = format!("{place}/{shown}");
        match kind {
            EntryKind::Symlink => {
                suites.problem(file, SYMLINK);
                continue;
            }
            EntryKind::Dir | EntryKind::Other => {
                suites.problem(file, "not a regular file");
                continue;
            }
            EntryKind::File => files = files.saturating_add(1),
        }
        if !is_slug(slug) {
            suites.problem(
                file,
                "a case name is 1 to 64 of a-z, 0-9 and -, starting with a letter or digit",
            );
            continue;
        }
        if suites.cases.len() >= MAX_CASES {
            suites.problem(
                file,
                format!("more than {MAX_CASES} cases; the rest are not read"),
            );
            continue;
        }
        match read_case(&path.join(&name), source)? {
            Ok((body, fixture)) => {
                valid = valid.saturating_add(1);
                suites.cases.push(Case {
                    skill: skill.to_owned(),
                    suite,
                    slug: slug.to_owned(),
                    file,
                    body,
                    fixture,
                });
            }
            Err(reason) => suites.problem(file, reason),
        }
    }
    if files == 0 {
        suites.problem(place, "the suite has no case");
    }
    Ok(valid)
}

/// The case file at `path`, parsed, with the fixture it names taken out of the body and
/// located under `source`'s suites, or why it is refused.
///
/// # Errors
/// [`LintError::Io`] when it cannot be read.
fn read_case(
    path: &Path,
    source: &Source<'_>,
) -> Result<Result<(Parsed, Option<Fixture>), String>, LintError> {
    let read = read_bounded(path).map_err(|error| LintError::io(path, error))?;
    let Some(bytes) = read.bytes else {
        return Ok(Err(format!(
            "larger than the read cap ({} bytes)",
            read.size
        )));
    };
    Ok(case::parse(&bytes)
        .map_err(|fault| fault.to_string())
        .and_then(|mut body| {
            let fixture = body
                .fixture
                .take()
                .map(|name| {
                    fixture::locate(source.dir, &name, &source.root)
                        .map(|dir| Fixture { name, dir })
                })
                .transpose()?;
            Ok((body, fixture))
        }))
}

/// Whether `text` is a case name: 1 to 64 of `a-z`, `0-9` and `-`, not starting with `-`.
fn is_slug(text: &str) -> bool {
    text.len() <= MAX_SLUG
        && text
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A directory entry's name for display.
fn display(name: &OsStr) -> String {
    name.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests;
