//! Where the OS will write a path that may not exist yet: [`resolve`].

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Most symlinks followed while resolving one path, as Linux's `MAXSYMLINKS`.
const MAX_LINK_HOPS: usize = 40;

/// Resolving a path met more than [`MAX_LINK_HOPS`] symlinks (a loop, or a long chain).
#[derive(Debug, thiserror::Error)]
#[error("more than {MAX_LINK_HOPS} symbolic links in {}", path.display())]
struct TooManyLinks {
    /// The path being resolved.
    path: PathBuf,
}

/// One component of a path still to resolve, owned so a link's target can join the queue.
#[derive(Debug)]
enum Step {
    /// A Windows prefix (`C:`, `\\?\C:`, `\\server\share`): starts the path again there.
    Prefix(OsString),
    /// The root directory: the path starts again at the root (keeping a Windows prefix).
    Root,
    /// `..`: the parent of what is resolved so far.
    Parent,
    /// A name.
    Name(OsString),
}

/// `path` (absolute) as the OS would reach it, component by component: each component is read
/// with `symlink_metadata` before the next, so `..` after a symlink climbs from the link's
/// target, not from its name, and a symlink is followed to its target whether that exists or
/// not (at most [`MAX_LINK_HOPS`] links); an existing component is canonicalised; a component
/// that does not exist is taken lexically (`.` dropped, `..` the parent, as a tool creating the
/// missing directories would reach it).
///
/// # Errors
/// The I/O error when a component cannot be read other than by not existing (permission, a
/// file used as a directory), or more than [`MAX_LINK_HOPS`] symlinks.
pub(crate) fn resolve(path: &Path) -> io::Result<PathBuf> {
    let mut pending = steps(path);
    let mut current = PathBuf::new();
    let mut hops = 0_usize;
    while let Some(step) = pending.pop() {
        match step {
            Step::Prefix(prefix) => current = PathBuf::from(prefix),
            Step::Root => current.push(Component::RootDir.as_os_str()),
            Step::Parent => {
                // At the root there is no parent: `/..` is `/`, as the OS reads it.
                current.pop();
            }
            Step::Name(name) => {
                let candidate = current.join(name);
                match inspect(&candidate)? {
                    Next::Resolved(real) => current = real,
                    Next::Missing => current = candidate,
                    Next::Link(target) => {
                        hops = hops.saturating_add(1);
                        if hops > MAX_LINK_HOPS {
                            return Err(io::Error::other(TooManyLinks {
                                path: path.to_path_buf(),
                            }));
                        }
                        // A relative target is read from the link's directory, `current`.
                        pending.extend(steps(&target));
                    }
                }
            }
        }
    }
    Ok(current)
}

/// What a name joined to a resolved directory turns out to be.
enum Next {
    /// It exists and is no symlink: its canonical path (Windows' long, verbatim form, as the
    /// worktree root is compared in).
    Resolved(PathBuf),
    /// Nothing is there.
    Missing,
    /// A symlink, live or dangling, with its target as written.
    Link(PathBuf),
}

/// Whether `candidate`, whose parent is resolved, is a symlink, exists, or is missing. Every
/// symlink is followed here rather than by `fs::canonicalize`, so [`MAX_LINK_HOPS`] bounds the
/// links followed on every platform.
fn inspect(candidate: &Path) -> io::Result<Next> {
    match fs::symlink_metadata(candidate) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::read_link(candidate).map(Next::Link)
        }
        Ok(_) => fs::canonicalize(candidate).map(Next::Resolved),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Next::Missing),
        Err(error) => Err(error),
    }
}

/// `path`'s components as steps, last first (a stack: `pop` gives the next one).
fn steps(path: &Path) -> Vec<Step> {
    path.components()
        .rev()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(Step::Prefix(prefix.as_os_str().to_owned())),
            Component::RootDir => Some(Step::Root),
            Component::CurDir => None,
            Component::ParentDir => Some(Step::Parent),
            Component::Normal(name) => Some(Step::Name(name.to_owned())),
        })
        .collect()
}

#[cfg(test)]
mod tests;
