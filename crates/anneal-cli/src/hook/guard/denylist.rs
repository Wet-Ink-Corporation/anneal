//! The deterministic denylist: it denies a command or leaves it undecided, never allows.
//!
//! The command is lexed by [`shell::parse`](super::shell::parse) into every simple command it
//! runs. Each one is unwrapped ([`wrappers`]: leading assignments and keywords, `sudo`, `env`,
//! `command`, `exec`, `nohup`, `time`, `nice`, `timeout`, `xargs`, `find -exec`, path prefixes)
//! and the strings that `bash -c`, `sh -c`, `zsh -c`, `eval` and `env -S` run are lexed and
//! checked in turn. Then the entries: [`git`] (force push to a protected branch, `git add -f`),
//! [`rm`] (recursive force delete outside temp and scratch) and [`config`] (Bash writes to lint
//! or hook config). See `docs/guard.md`.

mod config;
mod git;
mod rm;
mod wrappers;

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use super::shell::{self, SimpleCommand, Word};
use super::{Denial, Rule};
use wrappers::{Program, Run};

/// How deep `bash -c`, `eval`, `env -S` and `find -exec` may nest before a command is refused.
pub(crate) const MAX_DEPTH: usize = 8;

/// Directories under which `rm -rf` of a literal absolute path is allowed, besides `$TMPDIR`
/// and `$CLAUDE_PROJECT_DIR/.scratch`.
const TEMP_ROOTS: [&str; 4] = [
    "/tmp",
    "/var/folders",
    "/private/tmp",
    "/private/var/folders",
];

/// The project's scratch directory, under `$CLAUDE_PROJECT_DIR`.
const SCRATCH_DIR: &str = ".scratch";

/// What the rules need from the hook's environment.
#[derive(Debug)]
pub(crate) struct Context {
    /// Directories whose contents `rm -rf` may delete, normalized.
    safe_roots: Vec<PathBuf>,
}

impl Context {
    /// The context from the hook's environment: `$TMPDIR` and `$CLAUDE_PROJECT_DIR`, read
    /// through `lookup`. A value that is not an absolute path below the root is ignored.
    pub(crate) fn new(lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        let from_env = [
            lookup("TMPDIR").map(PathBuf::from),
            lookup("CLAUDE_PROJECT_DIR").map(|project| PathBuf::from(project).join(SCRATCH_DIR)),
        ];
        let safe_roots = TEMP_ROOTS
            .iter()
            .map(PathBuf::from)
            .chain(from_env.into_iter().flatten())
            .map(|root| normalize(&root))
            .filter(|root| root.has_root() && root.components().any(is_normal))
            .collect();
        Self { safe_roots }
    }

    /// Whether `path` (normalized) lies strictly inside a temporary or scratch directory.
    fn is_disposable(&self, path: &Path) -> bool {
        self.safe_roots
            .iter()
            .any(|root| path.starts_with(root) && path != root)
    }
}

/// The denylist's answer. It never allows: what it does not deny is for the fallback.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// On the denylist, or unreadable.
    Deny(Denial),
    /// Not on the denylist.
    Undecided,
}

/// Checks the Bash command `command` against the denylist.
pub(crate) fn check(command: &str, context: &Context) -> Verdict {
    match check_text(command, context, 0) {
        Some(denial) => Verdict::Deny(denial),
        None => Verdict::Undecided,
    }
}

/// Checks shell text at nesting `depth`.
fn check_text(text: &str, context: &Context, depth: usize) -> Option<Denial> {
    if depth > MAX_DEPTH {
        return Some(Denial::new(
            Rule::Unparseable,
            format!("the command nests bash -c, eval or find -exec deeper than {MAX_DEPTH}"),
        ));
    }
    match shell::parse(text) {
        Ok(commands) => commands
            .iter()
            .find_map(|command| check_command(command, context, depth)),
        Err(error) => Some(Denial::new(
            Rule::Unparseable,
            format!("the command cannot be read: {error}"),
        )),
    }
}

fn check_command(command: &SimpleCommand, context: &Context, depth: usize) -> Option<Denial> {
    command
        .writes
        .iter()
        .find_map(config::redirect)
        .or_else(|| check_words(&command.words, context, depth, false))
}

/// Checks one simple command's words; `appended` when `xargs` or `find -exec` adds operands
/// the guard cannot see.
fn check_words(words: &[Word], context: &Context, depth: usize, appended: bool) -> Option<Denial> {
    let inner = depth.saturating_add(1);
    match wrappers::unwrap(words) {
        Run::Nothing => None,
        Run::Script(text) => check_text(&text, context, inner),
        Run::Exec(words) => check_words(words, context, inner, true),
        Run::Command {
            program,
            args,
            appended: more,
        } => check_program(program, args, context, appended || more),
    }
}

/// Applies the entries for `program` (all of them when the program is not known).
fn check_program(
    program: Program<'_>,
    args: &[Word],
    context: &Context,
    appended: bool,
) -> Option<Denial> {
    match program {
        Program::Named("git") => git::check(args),
        Program::Named("rm") => {
            rm::check(args, context, appended).or_else(|| config::check("rm", args))
        }
        Program::Named(name) => config::check(name, args),
        Program::Unknown => git::check(args)
            .or_else(|| rm::check(args, context, appended))
            .or_else(|| config::check("cp", args)),
    }
}

/// `path` with `.` dropped and `..` applied lexically (symbolic links are not followed).
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    normal
}

const fn is_normal(component: Component<'_>) -> bool {
    matches!(component, Component::Normal(_))
}

#[cfg(test)]
mod tests;
