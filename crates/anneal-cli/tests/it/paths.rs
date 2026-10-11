//! `git_common_dir` finds the main repository's `.git` from anywhere, including a linked
//! worktree.

use std::error::Error;
use std::path::{Path, PathBuf};

use anneal_cli::paths::{PathError, git_common_dir};

use crate::support::repo_with_linked_worktree;

type TestResult = Result<(), Box<dyn Error>>;

fn canonical(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    Ok(std::fs::canonicalize(path)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn common_dir_of_the_main_worktree_is_its_dot_git() -> TestResult {
    let root = tempfile::tempdir()?;
    repo_with_linked_worktree(root.path());
    let main = root.path().join("main");

    let found = git_common_dir(&main)?;

    assert_eq!(
        canonical(&found)?,
        canonical(&main.join(".git"))?,
        "main .git"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn common_dir_from_a_linked_worktree_is_the_main_dot_git() -> TestResult {
    let root = tempfile::tempdir()?;
    repo_with_linked_worktree(root.path());

    let found = git_common_dir(&root.path().join("linked"))?;

    assert_eq!(
        canonical(&found)?,
        canonical(&root.path().join("main/.git"))?,
        "a linked worktree shares the main repository's ledger home"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn common_dir_from_a_subdirectory_resolves_against_it() -> TestResult {
    let root = tempfile::tempdir()?;
    repo_with_linked_worktree(root.path());
    let nested = root.path().join("linked/a/b");
    std::fs::create_dir_all(&nested)?;

    let found = git_common_dir(&nested)?;

    assert!(
        found.is_absolute(),
        "resolved to an absolute path: {found:?}"
    );
    assert_eq!(
        canonical(&found)?,
        canonical(&root.path().join("main/.git"))?,
        "same common dir from a subdirectory"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn outside_a_repository_is_not_a_repository() -> TestResult {
    let root = tempfile::tempdir()?;
    // A `.git` file pointing nowhere stops git here, even if a repository encloses the temp dir.
    std::fs::write(root.path().join(".git"), "gitdir: does-not-exist\n")?;

    let result = git_common_dir(root.path());

    assert!(
        matches!(result, Err(PathError::NotARepository { .. })),
        "got {result:?}"
    );
    Ok(())
}
