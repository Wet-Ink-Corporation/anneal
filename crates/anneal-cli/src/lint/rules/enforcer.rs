//! The enforcer a rule names, and whether it can be found in the repository.
//!
//! Three kinds, each written in backticks after `Enforcer:`:
//! - `anneal <command> [<subcommand>...] [args]`: a command of this binary, down to a command
//!   with no subcommands (a lint such as `anneal lint skills`, a gate such as `anneal plan check`);
//! - `hook <id>`: a hook id in `hooks.metadata.json`;
//! - `test <file>::<fn>`: a file under the repository root that defines `fn <fn>(`.

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use crate::lint::skills::LintError;
use crate::lint::skills::discover::MAX_READ_BYTES;

/// One enforcer as named, borrowed from the rule's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Enforcer<'a> {
    /// The words after `anneal`.
    Command(Vec<&'a str>),
    /// A hook id.
    Hook(&'a str),
    /// A test function in a file under the root.
    Test {
        /// The file, relative to the root.
        file: &'a str,
        /// The function's name.
        name: &'a str,
    },
}

/// Why a named enforcer cannot be found. Its `Display` completes "cannot be found: ...".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unresolved {
    /// The span starts with none of `anneal`, `hook` or `test`.
    UnknownKind,
    /// A word names no subcommand of the command before it.
    NoCommand {
        /// The command path that was found (`anneal lint`).
        path: String,
        /// The word that names none of its subcommands.
        word: String,
    },
    /// The words stop before a command with no subcommands.
    IncompleteCommand {
        /// The command path that was found.
        path: String,
    },
    /// `hook` is not followed by exactly one id.
    NotAHookId,
    /// The id is not in `hooks.metadata.json`.
    UnknownHook,
    /// `test` is not followed by exactly one `<file>::<fn>`.
    NotATestName,
    /// The test file is absolute, climbs out of the root, or resolves (through a symlink)
    /// outside it.
    OutsideRoot,
    /// No such file under the root.
    NoTestFile,
    /// The file is over the read cap.
    TestFileTooLarge,
    /// The file defines no such function.
    NoTestFn,
}

impl Display for Unresolved {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind => f.write_str("it starts with none of `anneal`, `hook` or `test`"),
            Self::NoCommand { path, word } => write!(f, "`{path}` has no command `{word}`"),
            Self::IncompleteCommand { path } => {
                write!(f, "`{path}` needs a subcommand to name one check")
            }
            Self::NotAHookId => f.write_str("write one hook id after `hook`"),
            Self::UnknownHook => f.write_str("no hook has that id in hooks.metadata.json"),
            Self::NotATestName => f.write_str("write `test <file>::<fn>`"),
            Self::OutsideRoot => {
                f.write_str("the test file is not a relative path that resolves under the root")
            }
            Self::NoTestFile => f.write_str("no such test file under the root"),
            Self::TestFileTooLarge => f.write_str("the test file is over the read cap"),
            Self::NoTestFn => f.write_str("the test file defines no such function"),
        }
    }
}

/// The repository root, resolved once (symlinks followed), so a test file can be required to
/// resolve inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Root(PathBuf);

impl Root {
    /// `path`, resolved.
    ///
    /// # Errors
    /// [`LintError::Io`] when it cannot be resolved.
    pub(crate) fn resolve(path: &Path) -> Result<Self, LintError> {
        fs::canonicalize(path)
            .map(Self)
            .map_err(|source| LintError::io(path, source))
    }

    /// Where `relative` (already known to be a plain relative path) leads once resolved,
    /// symlinks followed.
    ///
    /// # Errors
    /// [`LintError::Io`] when it exists but cannot be resolved.
    pub(crate) fn within(&self, relative: &Path) -> Result<Within, LintError> {
        let path = self.0.join(relative);
        match fs::canonicalize(&path) {
            Ok(resolved) if resolved.starts_with(&self.0) => Ok(Within::Inside(resolved)),
            Ok(_) => Ok(Within::Outside),
            Err(error) if is_absent(&error) => Ok(Within::Absent),
            Err(source) => Err(LintError::io(&path, source)),
        }
    }
}

/// Where a path under the root leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Within {
    /// To this resolved path inside the root.
    Inside(PathBuf),
    /// Out of the root, through a symlink.
    Outside,
    /// Nowhere: it does not exist.
    Absent,
}

/// What enforcers resolve against.
#[derive(Debug)]
pub(crate) struct Repository<'a> {
    /// The root test files are relative to.
    pub(crate) root: &'a Root,
    /// The ids of `hooks.metadata.json`.
    pub(crate) hook_ids: &'a BTreeSet<String>,
    /// This binary's command tree.
    pub(crate) commands: &'a clap::Command,
}

/// Reads one backticked enforcer.
pub(crate) fn parse(span: &str) -> Result<Enforcer<'_>, Unresolved> {
    let mut words = span.split_whitespace();
    match words.next() {
        Some("anneal") => Ok(Enforcer::Command(words.collect())),
        Some("hook") => match (words.next(), words.next()) {
            (Some(id), None) => Ok(Enforcer::Hook(id)),
            _ => Err(Unresolved::NotAHookId),
        },
        Some("test") => match (words.next(), words.next()) {
            (Some(target), None) => test_target(target).ok_or(Unresolved::NotATestName),
            _ => Err(Unresolved::NotATestName),
        },
        _ => Err(Unresolved::UnknownKind),
    }
}

/// `<file>::<fn>`, with a non-empty file and an identifier for the function.
fn test_target(target: &str) -> Option<Enforcer<'_>> {
    let (file, name) = target.rsplit_once("::")?;
    let identifier =
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (!file.is_empty() && identifier).then_some(Enforcer::Test { file, name })
}

/// Whether `enforcer` can be found in `repository`.
///
/// # Errors
/// [`LintError::Io`] when a test file exists but cannot be read.
pub(crate) fn resolve(
    enforcer: &Enforcer<'_>,
    repository: &Repository<'_>,
) -> Result<Result<(), Unresolved>, LintError> {
    match enforcer {
        Enforcer::Command(words) => Ok(command(repository.commands, words)),
        Enforcer::Hook(id) => Ok(if repository.hook_ids.contains(*id) {
            Ok(())
        } else {
            Err(Unresolved::UnknownHook)
        }),
        Enforcer::Test { file, name } => test(repository.root, file, name),
    }
}

/// Walks `words` down the command tree. `anneal` itself must be followed by a command, and a
/// command that requires a subcommand by one of its subcommands; the walk ends at a command
/// with no subcommands, or at one that runs without (`decide`, beside `decide label`) when the
/// next word names none of them.
fn command(root: &clap::Command, words: &[&str]) -> Result<(), Unresolved> {
    let mut current = root;
    let mut path = String::from("anneal");
    let mut words = words.iter();
    let mut required = true;
    while current.has_subcommands() {
        let word = words.next();
        let found = word.and_then(|word| current.find_subcommand(word).map(|next| (word, next)));
        let Some((word, next)) = found else {
            if !required {
                return Ok(());
            }
            return Err(match word {
                Some(word) => Unresolved::NoCommand {
                    path,
                    word: (*word).to_owned(),
                },
                None => Unresolved::IncompleteCommand { path },
            });
        };
        current = next;
        required = current.is_subcommand_required_set();
        path.push(' ');
        path.push_str(word);
    }
    Ok(())
}

/// Whether `<root>/<file>` defines `fn <name>(`. The file must resolve, symlinks followed,
/// inside the resolved root.
fn test(root: &Root, file: &str, name: &str) -> Result<Result<(), Unresolved>, LintError> {
    let relative = Path::new(file);
    let under_root = relative
        .components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if !under_root {
        return Ok(Err(Unresolved::OutsideRoot));
    }
    let resolved = match root.within(relative)? {
        Within::Inside(resolved) => resolved,
        Within::Outside => return Ok(Err(Unresolved::OutsideRoot)),
        Within::Absent => return Ok(Err(Unresolved::NoTestFile)),
    };
    let bytes = match read_test_file(&resolved) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(why)) => return Ok(Err(why)),
        Err(source) => return Err(LintError::io(&resolved, source)),
    };
    let defined = String::from_utf8_lossy(&bytes).contains(&format!("fn {name}("));
    Ok(if defined {
        Ok(())
    } else {
        Err(Unresolved::NoTestFn)
    })
}

/// A path that does not exist, or runs through a file as if it were a directory.
fn is_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

/// The bytes of the regular file at `path` (already resolved), read up to [`MAX_READ_BYTES`].
/// The path is checked before opening, so a FIFO is never opened (it would block), and the
/// opened handle again, in case the path was swapped in between.
fn read_test_file(path: &Path) -> io::Result<Result<Vec<u8>, Unresolved>> {
    if !fs::metadata(path)?.is_file() {
        return Ok(Err(Unresolved::NoTestFile));
    }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(Err(Unresolved::NoTestFile));
    }
    if metadata.len() > MAX_READ_BYTES {
        return Ok(Err(Unresolved::TestFileTooLarge));
    }
    let mut bytes = Vec::new();
    // One byte past the cap tells a file that grew since its size was read.
    file.take(MAX_READ_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let over = u64::try_from(bytes.len()).map_or(true, |read| read > MAX_READ_BYTES);
    Ok(if over {
        Err(Unresolved::TestFileTooLarge)
    } else {
        Ok(bytes)
    })
}
