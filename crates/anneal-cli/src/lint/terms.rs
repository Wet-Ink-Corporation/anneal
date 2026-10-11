//! `anneal lint terms`: hits of the glossary's avoided terms in a set of files.
//!
//! The glossary (`CONTEXT.md` by default) lists avoided terms on `_Avoid_: term, term` lines.
//! [`Glossary::scan`] is the pure check, [`lint`] the edge that reads the files. A hit is a
//! whole-word, case-insensitive use of a term; see `docs/define.md`.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::lint::skills::LintError;
use crate::lint::{Contents, read_named, read_whole};

/// Arguments of `anneal lint terms`.
#[derive(Debug, Clone, Args)]
pub struct TermsArgs {
    /// The glossary whose `_Avoid_: term, term` lines list the avoided terms
    #[arg(long, value_name = "FILE", default_value = "CONTEXT.md")]
    pub glossary: PathBuf,
    /// The files to scan, such as the changed files of a branch
    #[arg(value_name = "FILE", required = true)]
    pub files: Vec<PathBuf>,
}

/// What a `lint terms` run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Every hit, sorted by file, line and term.
    pub hits: Vec<TermHit>,
    /// How many avoided terms the glossary lists.
    pub terms: usize,
    /// How many files were scanned.
    pub checked: usize,
    /// Files too large to scan, listed so they are never passed silently.
    pub skipped: Vec<Skipped>,
}

/// A file not scanned because it is over the read cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    /// The file, as given.
    pub file: String,
    /// Its size in bytes.
    pub size: u64,
}

/// Scans `args.files` for the avoided terms of `args.glossary`.
///
/// Each file is named in output with its `.` components dropped, so a file named twice
/// (`a.md` and `./a.md`) is scanned once and shown as `a.md`; no other normalisation is done
/// (`..` and symlinks are kept as given). Files that are not UTF-8 are read lossily (a
/// replacement character is not a word character, so it creates no hit); files over the read
/// cap are listed in [`Outcome::skipped`].
///
/// # Errors
/// [`LintError::MissingPath`] or [`LintError::WrongKind`] when the glossary or a file is
/// missing or a directory, [`LintError::TooLarge`] when the glossary is over the read cap,
/// [`LintError::Io`] when a file cannot be read.
pub fn lint(args: &TermsArgs) -> Result<Outcome, LintError> {
    let glossary = Glossary::parse(&read_whole(&args.glossary)?);
    let files: BTreeSet<PathBuf> = args.files.iter().map(|path| without_dots(path)).collect();
    let mut outcome = Outcome {
        hits: Vec::new(),
        terms: glossary.terms().len(),
        checked: 0,
        skipped: Vec::new(),
    };
    for path in files {
        let file = path.display().to_string();
        match read_named(&path)? {
            Contents::Text(text) => {
                outcome.hits.extend(glossary.scan(&file, &text));
                outcome.checked = outcome.checked.saturating_add(1);
            }
            Contents::TooLarge(size) => outcome.skipped.push(Skipped { file, size }),
        }
    }
    outcome
        .hits
        .sort_by(|a, b| (&a.file, a.line, &a.term).cmp(&(&b.file, b.line, &b.term)));
    Ok(outcome)
}

/// The envelope's `data` for `lint terms`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    glossary: String,
    terms: usize,
    checked: usize,
    skipped: &'a [Skipped],
    hits: &'a [TermHit],
}

impl TermHit {
    /// Stable identity for baselines: `lint/terms:<file>:<line>:<term>`.
    #[must_use]
    pub fn key(&self) -> String {
        format!("lint/terms:{}:{}:{}", self.file, self.line, self.term)
    }
}

/// Writes the envelope for `outcome`, read with `glossary`, and returns its code: 1 when there
/// is any hit, else 0.
///
/// # Errors
/// [`CliError::Output`] when `out` cannot be written.
pub(crate) fn report<O: Write>(
    glossary: &Path,
    outcome: &Outcome,
    out: &mut O,
) -> Result<Code, CliError> {
    let glossary = glossary.display().to_string();
    let findings: Vec<Finding> = outcome
        .hits
        .iter()
        .map(|hit| {
            Finding::new(
                hit.key(),
                format!(
                    "{}:{}: error avoided term `{}` fix: use the term {glossary} defines instead.",
                    hit.file, hit.line, hit.term
                ),
            )
        })
        .collect();
    let code = if outcome.hits.is_empty() {
        exit_codes::LINT_CLEAN
    } else {
        exit_codes::LINT_FINDINGS
    };
    let data = Data {
        glossary,
        terms: outcome.terms,
        checked: outcome.checked,
        skipped: &outcome.skipped,
        hits: &outcome.hits,
    };
    Envelope::new(CommandName::Lint, Some("terms"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// `path` without its `.` components (`./a.md` is `a.md`), or `.` when nothing else is left.
/// Lexical only: `..` and symlinks are kept, since resolving them could name another file.
fn without_dots(path: &Path) -> PathBuf {
    let kept: PathBuf = path
        .components()
        .filter(|part| !matches!(part, Component::CurDir))
        .collect();
    if kept.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        kept
    }
}

/// One avoided term from the glossary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvoidedTerm {
    /// The term as written in the glossary.
    pub term: String,
    /// 1-based glossary line of its `_Avoid_:` entry.
    pub line: usize,
}

/// One use of an avoided term.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermHit {
    /// The file, as given.
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// The avoided term, as written in the glossary.
    pub term: String,
}

/// The avoided terms of a glossary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Glossary {
    terms: Vec<AvoidedTerm>,
}

/// The marker that starts a glossary line listing avoided terms.
pub const AVOID_MARKER: &str = "_Avoid_:";

impl Glossary {
    /// Reads every `_Avoid_: term, term` entry of `text`. Terms are split on `,` and trimmed
    /// of spaces, backticks and a final `.`; empty ones are skipped, and a term already listed
    /// (in any letter case) is dropped. Text before the marker on the line (a list bullet, the
    /// defined term) is ignored. Never fails: a glossary with no entry has no terms.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut terms: Vec<AvoidedTerm> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (index, line) in text.lines().enumerate() {
            let Some((_, list)) = line.split_once(AVOID_MARKER) else {
                continue;
            };
            for raw in list.split(',') {
                let term = clean(raw);
                if !term.is_empty() && seen.insert(term.to_lowercase()) {
                    terms.push(AvoidedTerm {
                        term,
                        line: index.saturating_add(1),
                    });
                }
            }
        }
        Self { terms }
    }

    /// The avoided terms, in glossary order.
    #[must_use]
    pub fn terms(&self) -> &[AvoidedTerm] {
        &self.terms
    }

    /// The hits of avoided terms in `text`, the contents of `file`: at most one per line and
    /// term, in line order then glossary order. A line holding [`AVOID_MARKER`] is a glossary
    /// entry, not a use, and is skipped.
    #[must_use]
    pub fn scan(&self, file: &str, text: &str) -> Vec<TermHit> {
        let needles: Vec<(String, &AvoidedTerm)> = self
            .terms
            .iter()
            .map(|t| (t.term.to_lowercase(), t))
            .collect();
        let mut hits = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.contains(AVOID_MARKER) {
                continue;
            }
            let lowered = line.to_lowercase();
            for (needle, avoided) in &needles {
                if contains_word(&lowered, needle) {
                    hits.push(TermHit {
                        file: file.to_owned(),
                        line: index.saturating_add(1),
                        term: avoided.term.clone(),
                    });
                }
            }
        }
        hits
    }
}

/// Whether `needle` occurs in `haystack` with a non-word character (or the line edge) on each
/// side. Every candidate position is tried, so a rejected match does not hide an overlapping
/// later one.
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(found) = haystack.get(from..).and_then(|rest| rest.find(needle)) {
        let start = from.saturating_add(found);
        let end = start.saturating_add(needle.len());
        let before = haystack.get(..start).and_then(|h| h.chars().next_back());
        let after = haystack.get(end..).and_then(|h| h.chars().next());
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            return true;
        }
        let step = haystack
            .get(start..)
            .and_then(|h| h.chars().next())
            .map_or(1, char::len_utf8);
        from = start.saturating_add(step);
    }
    false
}

/// A character that continues a word: Unicode alphanumeric or `_`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A term as listed: trimmed of spaces, backticks and a final `.`, with inner whitespace runs
/// collapsed to one space.
fn clean(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_end_matches('.')
        .trim()
        .trim_matches('`')
        .trim();
    trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
