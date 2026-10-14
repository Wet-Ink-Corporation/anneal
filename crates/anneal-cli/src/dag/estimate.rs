//! The per-ticket token estimate: a deterministic heuristic of how much context a ticket needs.
//!
//! `tokens = ceil(body characters / 4) + the cost of each named file`, where a named file is
//! one the tracker lists (markdown `files:`, as written) or a backticked path in the body
//! (`` `src/a.rs` ``, a `:line` suffix dropped; not a glob, a `/command` or a dotted identifier
//! such as `grill.confirmed`). A file costs `ceil(size / 4)` tokens, at most
//! [`MAX_FILE_TOKENS`], when it is a regular file under the root (relative, no `..`, not a
//! symlink, and its canonical path inside the canonical root, so a symlinked directory cannot
//! lead out); any other named file costs [`UNKNOWN_FILE_TOKENS`]. At most [`MAX_FILES`] files
//! are counted, and the sum saturates. Files are only sized, never read. See `docs/dag.md`.

use std::fs;
use std::path::{Component, Path};

use serde::Serialize;

use crate::dag::ticket::Ticket;

/// The single-context budget `dag frontier` flags against unless `--budget` says otherwise.
pub const DEFAULT_BUDGET: u64 = 100_000;
/// Characters per token, the usual rule of thumb for English text and code.
const CHARS_PER_TOKEN: u64 = 4;
/// The cost of a named file that cannot be sized under the root.
pub const UNKNOWN_FILE_TOKENS: u64 = 2_000;
/// The most one file can cost: a ticket rarely needs all of a very large file.
pub const MAX_FILE_TOKENS: u64 = 25_000;
/// The most files counted per ticket (H-13).
pub const MAX_FILES: usize = 64;

/// How a named file's cost was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// From its size under the root.
    Size,
    /// [`UNKNOWN_FILE_TOKENS`]: missing, outside the root, or not a regular file.
    Default,
}

/// One named file and its cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileCost {
    /// The path as named.
    pub path: String,
    /// Its tokens.
    pub tokens: u64,
    /// How they were found.
    pub basis: Basis,
}

/// A ticket's estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Estimate {
    /// Body plus files.
    pub tokens: u64,
    /// The body's share.
    pub body_tokens: u64,
    /// Each named file, in the order named.
    pub files: Vec<FileCost>,
}

/// Estimates `ticket`, sizing named files under `root`.
#[must_use]
pub fn estimate(ticket: &Ticket, root: &Path) -> Estimate {
    let chars = u64::try_from(ticket.body.chars().count()).unwrap_or(u64::MAX);
    let body_tokens = chars.div_ceil(CHARS_PER_TOKEN);
    // A root that cannot be resolved (missing, unreadable) sizes nothing: by the documented
    // rule every named file then costs the default, so the error has no other use.
    let canonical_root = fs::canonicalize(root);
    let files: Vec<FileCost> = named_files(ticket)
        .into_iter()
        .map(|path| file_cost(path, root, canonical_root.as_deref().ok()))
        .collect();
    let tokens = files
        .iter()
        .fold(body_tokens, |sum, file| sum.saturating_add(file.tokens));
    Estimate {
        tokens,
        body_tokens,
        files,
    }
}

/// The files `ticket` names: its listed files, then backticked paths in its body, each once,
/// at most [`MAX_FILES`].
fn named_files(ticket: &Ticket) -> Vec<&str> {
    let listed = ticket.files.iter().map(String::as_str);
    let mut named: Vec<&str> = Vec::new();
    for path in listed.chain(body_paths(&ticket.body)) {
        if named.len() == MAX_FILES {
            break;
        }
        if !named.contains(&path) {
            named.push(path);
        }
    }
    named
}

/// Backticked spans that look like a path: one word with a letter, and either a `/` or one of
/// [`KNOWN_EXTENSIONS`]; no option (`--x`), URL, glob (`*`, `?`, `[`) or command name (a
/// leading `/`, as `/orchestrate`). A trailing `:<line>` is dropped.
fn body_paths(body: &str) -> impl Iterator<Item = &str> {
    body.split('`').skip(1).step_by(2).filter_map(|span| {
        let path = span
            .rsplit_once(':')
            .filter(|(_, line)| !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()))
            .map_or(span, |(path, _)| path);
        let looks_like_path = !path.is_empty()
            && !path.contains(char::is_whitespace)
            && !path.starts_with(['-', '/'])
            && !path.contains("://")
            && !path.contains(['*', '?', '['])
            && path.contains(|c: char| c.is_ascii_alphabetic())
            && (path.contains('/') || has_known_extension(path));
        looks_like_path.then_some(path)
    })
}

/// The extensions that make a backticked word without a `/` a file name (compared in any
/// ASCII letter case), so a dotted identifier such as `grill.confirmed` is not one.
const KNOWN_EXTENSIONS: &[&str] = &[
    "bash", "bat", "c", "cc", "cfg", "cjs", "cmd", "conf", "cpp", "cs", "css", "csv", "go", "gql",
    "graphql", "h", "hpp", "html", "ini", "java", "js", "json", "jsonl", "jsx", "kt", "lock",
    "lua", "md", "mjs", "php", "proto", "ps1", "py", "rb", "rs", "rst", "scss", "sh", "sql", "svg",
    "swift", "toml", "ts", "tsv", "tsx", "txt", "xml", "yaml", "yml", "zsh",
];

fn has_known_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            KNOWN_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
        })
}

/// What `path` costs under `root`.
fn file_cost(path: &str, root: &Path, canonical_root: Option<&Path>) -> FileCost {
    let full = root.join(path);
    let contained = || {
        canonical_root.is_some_and(|canonical_root| {
            fs::canonicalize(&full).is_ok_and(|canonical| canonical.starts_with(canonical_root))
        })
    };
    let size = (under_root(path) && contained())
        .then(|| fs::symlink_metadata(&full))
        .and_then(Result::ok)
        .filter(fs::Metadata::is_file)
        .map(|metadata| metadata.len());
    let (tokens, basis) = match size {
        Some(bytes) => (
            bytes.div_ceil(CHARS_PER_TOKEN).min(MAX_FILE_TOKENS),
            Basis::Size,
        ),
        // Missing, unreadable, outside the root or not a file: the documented default.
        None => (UNKNOWN_FILE_TOKENS, Basis::Default),
    };
    FileCost {
        path: path.to_owned(),
        tokens,
        basis,
    }
}

/// True when `path` is relative and stays under its root: only normal components and `.`.
fn under_root(path: &str) -> bool {
    let mut components = Path::new(path).components().peekable();
    components.peek().is_some()
        && components.all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests;
