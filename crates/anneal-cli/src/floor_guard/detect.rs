//! Turning a parsed diff into floor findings. Pure: the caller reads the working tree.
//!
//! - [`suppression`]: suppressions on added lines (prose files are skipped).
//! - [`test_files`]: test paths and test markers: deleted and emptied test files, and more
//!   tests removed than added across the whole diff (so a test moved between files is not a
//!   removal).
//! - [`threshold`]: numeric thresholds whose new value, paired by key with a removed line of
//!   the same hunk, loosens the floor.

mod suppression;
mod test_files;
mod threshold;

use serde::Serialize;

use super::diff::{FileDiff, Hunk, Status};
use test_files::{is_test_marker, is_test_path};
use threshold::{Kind, Pair};

/// Most findings kept; the rest are counted in [`Report::omitted`].
pub(crate) const MAX_FINDINGS: usize = 1_000;

/// What kind of floor was lowered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// A lint or test suppression was added.
    Suppression,
    /// A test file was deleted.
    DeletedTest,
    /// A test file was emptied.
    EmptiedTest,
    /// More tests were removed than added.
    RemovedTest,
    /// A coverage floor was lowered.
    CoverageLowered,
    /// A timeout was raised.
    TimeoutRaised,
    /// A ratchet value moved the loose way.
    RatchetLoosened,
}

impl Category {
    /// The name in finding keys and JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Suppression => "suppression",
            Self::DeletedTest => "deleted_test",
            Self::EmptiedTest => "emptied_test",
            Self::RemovedTest => "removed_test",
            Self::CoverageLowered => "coverage_lowered",
            Self::TimeoutRaised => "timeout_raised",
            Self::RatchetLoosened => "ratchet_loosened",
        }
    }
}

/// One lowered floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FloorFinding {
    /// What kind.
    pub category: Category,
    /// The file, relative to the repository root.
    pub file: String,
    /// The line: in the working tree, except for `removed_test`, where it is the first removed
    /// test's line in the base; `None` for a whole file (`deleted_test`, `emptied_test`).
    pub line: Option<u64>,
    /// One line for a human.
    pub message: String,
}

impl FloorFinding {
    /// The stable key: `floor/<category>:<file>[:<line>]`.
    #[must_use]
    pub fn key(&self) -> String {
        match self.line {
            Some(line) => format!("floor/{}:{}:{line}", self.category.as_str(), self.file),
            None => format!("floor/{}:{}", self.category.as_str(), self.file),
        }
    }
}

/// What the detector needs to know of a file's working-tree content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostImage {
    blank: bool,
    coverage_threshold: bool,
}

impl PostImage {
    /// Reads what matters from `content`.
    pub(crate) fn of(content: &str) -> Self {
        Self {
            blank: content.trim().is_empty(),
            coverage_threshold: content.contains("coverageThreshold"),
        }
    }
}

/// The findings, at most [`MAX_FINDINGS`], and how many more there were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) findings: Vec<FloorFinding>,
    pub(crate) omitted: usize,
}

/// The test markers one file removed and added.
#[derive(Debug)]
struct Tally {
    file: String,
    first_removed: Option<u64>,
    removed: usize,
    added: usize,
    /// Already reported as a deleted or emptied test file.
    reported: bool,
}

/// Collects findings file by file; removed tests are judged across the whole diff in
/// [`Detector::finish`].
#[derive(Debug, Default)]
pub(crate) struct Detector {
    findings: Vec<FloorFinding>,
    tallies: Vec<Tally>,
}

impl Detector {
    /// Checks one file; `post` is its working-tree content when it has one.
    pub(crate) fn file(&mut self, diff: &FileDiff<'_>, post: Option<&PostImage>) {
        self.suppressions(diff);
        let reported = self.test_file(diff, post);
        self.tally(diff, reported);
        if diff.status != Status::Deleted {
            let coverage_threshold = post.is_some_and(|post| post.coverage_threshold);
            for hunk in &diff.hunks {
                self.thresholds(&diff.path, hunk, coverage_threshold);
            }
        }
    }

    /// Records a finding of `category` at `line` of `file`.
    fn push(&mut self, category: Category, (file, line): (&str, Option<u64>), message: String) {
        self.findings.push(FloorFinding {
            category,
            // The finding owns the path it reports.
            file: file.to_owned(),
            line,
            message,
        });
    }

    fn suppressions(&mut self, diff: &FileDiff<'_>) {
        if suppression::is_prose(&diff.path) {
            return;
        }
        for line in diff.hunks.iter().flat_map(|hunk| &hunk.added) {
            if let Some(pattern) = suppression::suppression(line.text) {
                let message = format!("adds a suppression ({pattern})");
                let at = (diff.path.as_ref(), Some(line.number));
                self.push(Category::Suppression, at, message);
            }
        }
    }

    /// Reports a deleted or emptied test file; true when it did.
    fn test_file(&mut self, diff: &FileDiff<'_>, post: Option<&PostImage>) -> bool {
        if diff.status == Status::Deleted {
            if !is_test_path(&diff.old_path) {
                return false;
            }
            let message = "deletes a test file".to_owned();
            self.push(Category::DeletedTest, (&diff.old_path, None), message);
            return true;
        }
        let removed_any = diff.hunks.iter().any(|hunk| !hunk.removed.is_empty());
        let emptied = diff.status != Status::Added
            && removed_any
            && is_test_path(&diff.path)
            && post.is_some_and(|post| post.blank);
        if emptied {
            let message = "empties a test file".to_owned();
            self.push(Category::EmptiedTest, (&diff.path, None), message);
        }
        emptied
    }

    fn tally(&mut self, diff: &FileDiff<'_>, reported: bool) {
        let removed: Vec<u64> = diff
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.removed)
            .filter(|line| is_test_marker(&diff.old_path, line.text))
            .map(|line| line.number)
            .collect();
        let added = diff
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.added)
            .filter(|line| is_test_marker(&diff.path, line.text))
            .count();
        if removed.is_empty() && added == 0 {
            return;
        }
        self.tallies.push(Tally {
            // The tally owns the path it may report.
            file: diff.path.as_ref().to_owned(),
            first_removed: removed.first().copied(),
            removed: removed.len(),
            added,
            reported,
        });
    }

    /// Pairs each added threshold with the first unpaired removed one of the same key in
    /// `hunk`, and reports those that loosen.
    fn thresholds(&mut self, path: &str, hunk: &Hunk<'_>, coverage_threshold: bool) {
        let typed = |pair: &Pair<'_>| threshold::kind(path, pair.key, coverage_threshold);
        let mut old: Vec<Option<Pair<'_>>> = hunk
            .removed
            .iter()
            .flat_map(|line| threshold::pairs(line.text))
            .filter(|pair| typed(pair).is_some())
            .map(Some)
            .collect();
        for line in &hunk.added {
            for new in threshold::pairs(line.text) {
                let Some(kind) = typed(&new) else { continue };
                let Some(paired) = old
                    .iter_mut()
                    .find(|old| old.is_some_and(|old| old.key == new.key))
                    .and_then(Option::take)
                else {
                    continue;
                };
                if kind.loosened(paired.value, new.value) {
                    let (category, verb) = match kind {
                        Kind::Coverage => (Category::CoverageLowered, "lowers"),
                        Kind::Timeout => (Category::TimeoutRaised, "raises"),
                        Kind::RatchetMax | Kind::RatchetMin => {
                            (Category::RatchetLoosened, "loosens")
                        }
                    };
                    let message =
                        format!("{verb} {} from {} to {}", new.key, paired.value, new.value);
                    self.push(category, (path, Some(line.number)), message);
                }
            }
        }
    }

    /// The findings, with removed tests judged across the diff, sorted by file, line and
    /// category, and bounded.
    pub(crate) fn finish(mut self) -> Report {
        let removed: usize = self.tallies.iter().map(|tally| tally.removed).sum();
        let added: usize = self.tallies.iter().map(|tally| tally.added).sum();
        if removed > added {
            let net = removed.saturating_sub(added);
            for tally in std::mem::take(&mut self.tallies) {
                if tally.reported || tally.removed <= tally.added {
                    continue;
                }
                let message = format!(
                    "removes {} test(s) and adds {} here; the diff has {net} fewer test(s)",
                    tally.removed, tally.added
                );
                self.findings.push(FloorFinding {
                    category: Category::RemovedTest,
                    file: tally.file,
                    line: tally.first_removed,
                    message,
                });
            }
        }
        let mut findings = self.findings;
        findings.sort_by(|a, b| (&a.file, a.line, a.category).cmp(&(&b.file, b.line, b.category)));
        let omitted = findings.len().saturating_sub(MAX_FINDINGS);
        findings.truncate(MAX_FINDINGS);
        Report { findings, omitted }
    }
}

#[cfg(test)]
mod tests;
