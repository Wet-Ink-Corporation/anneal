//! Which files the preconditions hook guards, and the key that names a worktree. Paths are
//! compared as text: `\` reads as `/`, `.` and `..` are taken lexically (`mode` resolves
//! symlinks before calling in), and ASCII letters match in any case, as the guard's protected
//! paths do (APFS and NTFS are case-insensitive by default).

use crate::context::note::CONTINUE_HERE;
use crate::floor_guard::is_test_path;

/// Top-level directories of the worktree that hold plans, ledger state and scratch work, never
/// source.
const SCRATCH_DIRS: [&str; 4] = [".temper", ".anneal", "tmp", "scratch"];

/// Extensions of prose documents, never source.
const DOC_EXTENSIONS: [&str; 3] = ["md", "txt", "rst"];

/// Directories, at any depth, that tools fill while code runs (bytecode, test and lint
/// caches, npm's logs), never source. Lowercase. Virtual environments are left out: code
/// written into one runs.
const GENERATED_DIRS: [&str; 7] = [
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".hypothesis",
    ".npm",
    ".cache",
];

/// Extensions of files tools write while code runs: Python bytecode and logs. Lowercase.
const GENERATED_EXTENSIONS: [&str; 3] = ["pyc", "pyo", "log"];

/// Extensions of code and of configuration tools read on import, source even under one of
/// [`GENERATED_DIRS`], so writing code into a cache directory does not skip the guard.
/// Lowercase. `json` is left out: mypy writes its cache as `*.data.json` and `*.meta.json`.
const SOURCE_EXTENSIONS: [&str; 15] = [
    "py", "js", "ts", "mjs", "cjs", "sh", "rs", "rb", "go", "java", "toml", "yaml", "yml", "cfg",
    "ini",
];

/// What a path inside the worktree is, for the preconditions hook. Only [`PathKind::Source`]
/// is guarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathKind {
    /// The plan or the design tree recorded when build mode was entered.
    Artifact,
    /// Under `.temper/`, `.anneal/`, `tmp/` or `scratch/` at the worktree root, or the handoff
    /// note `.continue-here` there.
    Scratch,
    /// What tools write while code runs: anything under a cache directory such as
    /// `__pycache__/` or `.pytest_cache/` at any depth, Python bytecode (`*.pyc`, `*.pyo`) and
    /// logs (`*.log`).
    Generated,
    /// A prose document: `*.md`, `*.txt` or `*.rst`.
    Doc,
    /// A test file, as `floor-guard` reads one.
    Test,
    /// Anything else.
    Source,
}

/// `path` with `\` read as `/`, empty and `.` components dropped and `..` taken lexically. A
/// leading `/` (or the `//` of a UNC path) and a drive (`C:`) are kept, and `..` never climbs
/// above them; a relative path keeps the `..` it starts with.
pub(crate) fn normalise(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let root = if slashed.starts_with("//") {
        "//"
    } else if slashed.starts_with('/') {
        "/"
    } else {
        ""
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in slashed.split('/') {
        match part {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&last) if last != ".." && !is_drive(last) => {
                    parts.pop();
                }
                Some(&"..") => parts.push(".."),
                None if root.is_empty() => parts.push(".."),
                Some(_) | None => {}
            },
            name => parts.push(name),
        }
    }
    format!("{root}{}", parts.join("/"))
}

/// True for a Windows drive such as `C:`.
fn is_drive(part: &str) -> bool {
    matches!(part.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic())
}

/// `path` relative to `root`, both normalised (see [`normalise`]), with ASCII letters compared
/// in any case; `""` for the root itself and `None` for a path outside it.
pub(crate) fn relative<'p>(root: &str, path: &'p str) -> Option<&'p str> {
    let root = root.trim_end_matches('/');
    let head = path.get(..root.len())?;
    if !head.eq_ignore_ascii_case(root) {
        return None;
    }
    let rest = path.get(root.len()..)?;
    if rest.is_empty() {
        return Some(rest);
    }
    rest.strip_prefix('/')
}

/// The kind of `path`, relative to the worktree root and `/`-separated; `artifacts` are the
/// build's plan and design tree, relative the same way.
pub(crate) fn classify(path: &str, artifacts: &[&str]) -> PathKind {
    if artifacts
        .iter()
        .any(|artifact| artifact.eq_ignore_ascii_case(path))
    {
        return PathKind::Artifact;
    }
    let lower = path.to_ascii_lowercase();
    if let Some((top, _)) = lower.split_once('/')
        && SCRATCH_DIRS.contains(&top)
    {
        return PathKind::Scratch;
    }
    if lower == CONTINUE_HERE {
        return PathKind::Scratch;
    }
    let extension = lower.rsplit_once('.').map(|(_, extension)| extension);
    if is_generated(&lower, extension) {
        return PathKind::Generated;
    }
    if extension.is_some_and(|extension| DOC_EXTENSIONS.contains(&extension)) {
        return PathKind::Doc;
    }
    if is_test_path(&lower) || is_rust_test_module(&lower) {
        return PathKind::Test;
    }
    PathKind::Source
}

/// True when the lowercase `path`, whose last `.`-suffix is `extension`, is what a tool wrote
/// while code ran: its extension is one of [`GENERATED_EXTENSIONS`], or a directory on the way
/// to it is one of [`GENERATED_DIRS`] and its file name's extension is not one of
/// [`SOURCE_EXTENSIONS`].
fn is_generated(path: &str, extension: Option<&str>) -> bool {
    if extension.is_some_and(|extension| GENERATED_EXTENSIONS.contains(&extension)) {
        return true;
    }
    let Some((dirs, name)) = path.rsplit_once('/') else {
        return false;
    };
    let source_like = name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| SOURCE_EXTENSIONS.contains(&extension));
    !source_like && dirs.split('/').any(|dir| GENERATED_DIRS.contains(&dir))
}

/// True for a Rust test module file, `tests.rs` or `test.rs` (`#[cfg(test)] mod tests;`).
/// Only the hook counts these: floor-guard's `is_test_path` stays as it is.
fn is_rust_test_module(path: &str) -> bool {
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    name == "tests.rs" || name == "test.rs"
}

/// The value that names a worktree in the ledger (`EventEnvelope::worktree`, tag
/// `worktree:`): its top-level directory with `/` separators, and on Windows and macOS, whose
/// file systems ignore letter case by default, in ASCII lowercase, so every spelling of one
/// directory gives one key. `anneal mode` writes it and the hook reads by it.
pub(crate) fn worktree_key(root: &str) -> String {
    let slashed = root.replace('\\', "/");
    if cfg!(any(windows, target_os = "macos")) {
        slashed.to_ascii_lowercase()
    } else {
        slashed
    }
}

#[cfg(test)]
mod tests;
