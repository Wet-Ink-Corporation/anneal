//! D5's deterministic triggers, which send a change to the doubt review whatever Clef says: a
//! path (or the path it was renamed from) matching one of `ship`'s default auth, payments or
//! migrations globs, more than [`LINE_TRIGGER`] changed lines, or an untracked file too large to
//! count. Pure: the change is read by `floor_guard::change`.

use serde::Serialize;

use crate::floor_guard::change::Touched;
use crate::glob::{Glob, GlobError};
use crate::ship::skip_rule::{DEFAULT_SENSITIVE, MAX_LISTED};

/// A change with more changed lines than this (added plus removed) is reviewed.
pub(crate) const LINE_TRIGGER: u64 = 400;

/// The categories of `ship`'s default sensitive globs that are D5 triggers.
const CATEGORIES: [&str; 3] = ["auth", "payments", "migrations"];

/// A sensitive glob D5 applies, with its category.
#[derive(Debug, Clone)]
pub(crate) struct Rule {
    glob: Glob,
    pattern: &'static str,
    category: &'static str,
}

/// The globs of `ship`'s defaults in [`CATEGORIES`], in their order.
///
/// # Errors
/// The [`GlobError`] of a default glob that does not compile (a test keeps them valid).
pub(crate) fn rules() -> Result<Vec<Rule>, GlobError> {
    DEFAULT_SENSITIVE
        .iter()
        .filter(|(category, _)| CATEGORIES.contains(category))
        .map(|&(category, pattern)| {
            Glob::new(pattern).map(|glob| Rule {
                glob,
                pattern,
                category,
            })
        })
        .collect()
}

/// Why a change goes to the doubt review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "trigger", rename_all = "snake_case")]
pub(crate) enum Trigger {
    /// A file matches an auth, payments or migrations glob.
    Path {
        /// The path that matched (the one it was renamed from, when only that did).
        path: String,
        /// The first glob it matches, as written.
        glob: &'static str,
        /// The glob's category.
        category: &'static str,
    },
    /// More than [`LINE_TRIGGER`] lines changed.
    Lines {
        /// How many, added plus removed.
        lines: u64,
        /// [`LINE_TRIGGER`].
        limit: u64,
    },
    /// An untracked file too large to count its lines.
    Unmeasured {
        /// The file.
        path: String,
    },
}

/// What the triggers found in a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Triggers {
    /// The triggers that fired: files first (at most `MAX_LISTED`), then the line count.
    pub(crate) fired: Vec<Trigger>,
    /// File triggers past the listing cap, counted rather than listed (H-13).
    pub(crate) omitted: usize,
    /// The change's changed lines, files that could not be counted left out.
    pub(crate) lines: u64,
}

/// The triggers `files` fire under `rules`.
pub(crate) fn judge(files: &[Touched], rules: &[Rule]) -> Triggers {
    let mut fired = Vec::new();
    let mut omitted = 0_usize;
    let mut lines = 0_u64;
    for file in files {
        lines = lines.saturating_add(file.lines.unwrap_or(0));
        let trigger = sensitive(file, rules).or_else(|| {
            file.lines.is_none().then(|| Trigger::Unmeasured {
                // The report owns the paths it lists.
                path: file.path.clone(),
            })
        });
        match trigger {
            Some(trigger) if fired.len() < MAX_LISTED => fired.push(trigger),
            Some(_) => omitted = omitted.saturating_add(1),
            None => {}
        }
    }
    if lines > LINE_TRIGGER {
        fired.push(Trigger::Lines {
            lines,
            limit: LINE_TRIGGER,
        });
    }
    Triggers {
        fired,
        omitted,
        lines,
    }
}

/// The path trigger of `file`: its path, else the one it was renamed from, against `rules` in
/// order.
fn sensitive(file: &Touched, rules: &[Rule]) -> Option<Trigger> {
    let paths = std::iter::once(file.path.as_str()).chain(file.old_path.as_deref());
    for path in paths {
        if let Some(rule) = rules.iter().find(|rule| rule.glob.matches(path)) {
            return Some(Trigger::Path {
                // The report owns the paths it lists.
                path: path.to_owned(),
                glob: rule.pattern,
                category: rule.category,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests;
