//! The skip-rule: whether a change may skip full review.
//!
//! A change may skip it when its diff from the base touches at most `MAX_FILES` (2) files,
//! fewer than `LINE_LIMIT` (50) lines (added plus deleted), no binary file and no path matching
//! a sensitive glob (`DEFAULT_SENSITIVE`, plus the repository's `ship.sensitive_globs`). Pure:
//! the diff is read by `pr_diff::numstat`.

use std::num::ParseIntError;

use serde::Serialize;

use crate::glob::{Glob, GlobError};

/// The most files a change may touch and skip full review.
pub(crate) const MAX_FILES: usize = 2;

/// A change skips full review only with fewer changed lines than this.
pub(crate) const LINE_LIMIT: u64 = 50;

/// The most binary and sensitive files listed; the rest are counted in `omitted` (H-13).
pub(crate) const MAX_LISTED: usize = 64;

/// The category of a configured glob in a report.
const CONFIGURED: &str = "configured";

/// The default sensitive globs and their categories: auth, payments, migrations, CI, infra,
/// and the gate's own configuration, so editing `.anneal/config.json` (which can switch the
/// review requirement) is never a small change. Matched in this order; the first glob that
/// matches a path is the one reported.
pub(crate) const DEFAULT_SENSITIVE: &[(&str, &str)] = &[
    ("auth", "**/*auth*"),
    ("auth", "**/*auth*/**"),
    ("auth", "**/*login*"),
    ("auth", "**/*login*/**"),
    ("auth", "**/*password*"),
    ("auth", "**/*password*/**"),
    ("auth", "**/*credential*"),
    ("auth", "**/*credential*/**"),
    ("auth", "**/*secret*"),
    ("auth", "**/*secret*/**"),
    ("payments", "**/*payment*"),
    ("payments", "**/*payment*/**"),
    ("payments", "**/*billing*"),
    ("payments", "**/*billing*/**"),
    ("payments", "**/*invoice*"),
    ("payments", "**/*invoice*/**"),
    ("payments", "**/*checkout*"),
    ("payments", "**/*checkout*/**"),
    ("payments", "**/*stripe*"),
    ("payments", "**/*stripe*/**"),
    ("migrations", "**/migrations/**"),
    ("migrations", "**/migrate/**"),
    ("migrations", "**/*migration*"),
    ("migrations", "**/*.sql"),
    ("ci", ".github/**"),
    ("ci", ".gitlab-ci.yml"),
    ("ci", ".gitlab/**"),
    ("ci", ".circleci/**"),
    ("ci", ".buildkite/**"),
    ("ci", ".travis.yml"),
    ("ci", "azure-pipelines.yml"),
    ("ci", "**/Jenkinsfile"),
    ("infra", "**/*.tf"),
    ("infra", "**/*.tfvars"),
    ("infra", "**/terraform/**"),
    ("infra", "**/infra/**"),
    ("infra", "**/infrastructure/**"),
    ("infra", "**/k8s/**"),
    ("infra", "**/kubernetes/**"),
    ("infra", "**/helm/**"),
    ("infra", "**/ansible/**"),
    ("infra", "**/deploy/**"),
    ("infra", "**/Dockerfile*"),
    ("infra", "**/docker-compose*"),
    ("gate config", ".anneal/**"),
    ("gate config", ".claude/**"),
];

/// One file in the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileChange {
    /// Its path relative to the repository root, `/`-separated.
    pub(crate) path: String,
    /// Lines added plus deleted; `None` for a binary file.
    pub(crate) lines: Option<u64>,
}

/// A sensitive glob and its category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sensitive {
    glob: Glob,
    category: &'static str,
}

/// The default sensitive globs, then `configured` (category `configured`).
///
/// # Errors
/// The [`GlobError`] of a default glob that does not compile (a test keeps them valid).
pub(crate) fn sensitive_rules(configured: Vec<Glob>) -> Result<Vec<Sensitive>, GlobError> {
    let defaults = DEFAULT_SENSITIVE
        .iter()
        .map(|&(category, pattern)| Glob::new(pattern).map(|glob| Sensitive { glob, category }));
    let configured = configured.into_iter().map(|glob| {
        Ok(Sensitive {
            glob,
            category: CONFIGURED,
        })
    });
    defaults.chain(configured).collect()
}

/// Why a change needs a full review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub(crate) enum Reason {
    /// More than [`MAX_FILES`] files changed.
    TooManyFiles {
        /// How many.
        files: usize,
    },
    /// [`LINE_LIMIT`] or more lines changed.
    TooManyLines {
        /// How many, added plus deleted, binary files not counted.
        lines: u64,
    },
    /// A binary file changed: its size in lines is unknown.
    Binary {
        /// The file.
        path: String,
    },
    /// A file matches a sensitive glob.
    Sensitive {
        /// The file.
        path: String,
        /// The first glob it matches, as written.
        glob: String,
        /// The glob's category.
        category: &'static str,
    },
}

/// The skip-rule's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkipDecision {
    /// Files changed.
    pub(crate) files: usize,
    /// Lines changed, added plus deleted, binary files not counted.
    pub(crate) lines: u64,
    /// Why a full review is needed; empty when the change may skip it.
    pub(crate) reasons: Vec<Reason>,
    /// Binary and sensitive files past [`MAX_LISTED`], counted but not listed.
    pub(crate) omitted: usize,
}

impl SkipDecision {
    /// Whether the change may skip full review.
    pub(crate) fn skip(&self) -> bool {
        self.reasons.is_empty() && self.omitted == 0
    }
}

/// Judges `changes` against the rule and the sensitive globs `rules`.
pub(crate) fn judge(changes: &[FileChange], rules: &[Sensitive]) -> SkipDecision {
    let files = changes.len();
    let lines = changes
        .iter()
        .filter_map(|change| change.lines)
        .fold(0_u64, u64::saturating_add);
    let mut reasons = Vec::new();
    if files > MAX_FILES {
        reasons.push(Reason::TooManyFiles { files });
    }
    if lines >= LINE_LIMIT {
        reasons.push(Reason::TooManyLines { lines });
    }
    let mut listed = 0_usize;
    let mut omitted = 0_usize;
    for change in changes {
        let Some(reason) = file_reason(change, rules) else {
            continue;
        };
        if listed < MAX_LISTED {
            listed = listed.saturating_add(1);
            reasons.push(reason);
        } else {
            omitted = omitted.saturating_add(1);
        }
    }
    SkipDecision {
        files,
        lines,
        reasons,
        omitted,
    }
}

/// Why `change` alone needs a full review: it is binary, or the first sensitive glob it matches.
fn file_reason(change: &FileChange, rules: &[Sensitive]) -> Option<Reason> {
    if change.lines.is_none() {
        return Some(Reason::Binary {
            path: change.path.clone(),
        });
    }
    rules
        .iter()
        .find(|rule| rule.glob.matches(&change.path))
        .map(|rule| Reason::Sensitive {
            // The report owns the path and glob it names.
            path: change.path.clone(),
            glob: rule.glob.as_str().to_owned(),
            category: rule.category,
        })
}

/// Why `git diff --numstat -z` output could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NumstatError {
    /// A record is not `<added>\t<deleted>\t<path>`.
    #[error("git diff --numstat printed a record it does not document: {record:?}")]
    Malformed {
        /// The record, lossy UTF-8.
        record: String,
        /// Why a count did not read as a number, when that is what is wrong.
        #[source]
        source: Option<ParseIntError>,
    },
}

/// The files of `git diff --numstat -z --no-renames` output: NUL-terminated records of
/// `<added>\t<deleted>\t<path>`, `-` for both counts of a binary file.
///
/// # Errors
/// [`NumstatError::Malformed`] for a record that is not of that shape.
pub(crate) fn parse_numstat(output: &[u8]) -> Result<Vec<FileChange>, NumstatError> {
    output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(parse_record)
        .collect()
}

fn parse_record(record: &[u8]) -> Result<FileChange, NumstatError> {
    let text = String::from_utf8_lossy(record);
    let malformed = |source| NumstatError::Malformed {
        record: text.clone().into_owned(),
        source,
    };
    let mut fields = text.splitn(3, '\t');
    let (Some(added), Some(deleted), Some(path)) = (fields.next(), fields.next(), fields.next())
    else {
        return Err(malformed(None));
    };
    let lines = match (added, deleted) {
        ("-", "-") => None,
        (added, deleted) => {
            let count = |field: &str| field.parse::<u64>().map_err(|error| malformed(Some(error)));
            Some(count(added)?.saturating_add(count(deleted)?))
        }
    };
    if path.is_empty() {
        return Err(malformed(None));
    }
    Ok(FileChange {
        path: path.to_owned(),
        lines,
    })
}

#[cfg(test)]
mod tests;
