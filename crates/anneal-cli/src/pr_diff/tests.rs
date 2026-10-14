use std::time::Duration;

use super::{
    Budget, DiffError, TextLimits, default_base, diff_text, is_ancestor, numstat, patch_id,
    resolve_commit,
};
use crate::test_repo::{ScratchRepo, Setup};

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_rebase_only_change_keeps_the_patch_id_and_a_real_change_does_not() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let first = repo.feature()?;
    let before = patch_id(repo.path(), "main", &first, TIMEOUT)?;

    let rebased = repo.rebase_feature()?;
    let after_rebase = patch_id(repo.path(), "main", &rebased, TIMEOUT)?;
    let changed = repo.change_feature()?;
    let after_change = patch_id(repo.path(), "main", &changed, TIMEOUT)?;

    assert_ne!(first, rebased, "the rebase made a new commit");
    assert!(before.is_some(), "a non-empty diff has a patch id");
    assert_eq!(before, after_rebase, "rebase only: same patch id");
    assert_ne!(before, after_change, "a real change: another patch id");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_whitespace_only_change_changes_the_patch_id() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let first = repo.feature()?;
    let before = patch_id(repo.path(), "main", &first, TIMEOUT)?;
    repo.write("src/lib.rs", "pub fn answer() -> u32 {\n    42 \n}\n")?;
    let spaced = repo.commit("trailing space")?;

    let after = patch_id(repo.path(), "main", &spaced, TIMEOUT)?;

    assert_ne!(
        before, after,
        "whitespace is code (Python, YAML, Makefiles): --verbatim keeps it in the hash"
    );
    Ok(())
}

/// Branch `feature` from `main` adding a submodule entry (a gitlink) at `sub` pointing at
/// `commit`, with `diff.ignoreSubmodules=all` set as a repository might; returns its head.
fn gitlink_feature(repo: &ScratchRepo, commit: &str) -> Setup<String> {
    repo.git(&["config", "diff.ignoreSubmodules", "all"])?;
    repo.git(&["checkout", "-q", "-b", "feature"])?;
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{commit},sub"),
    ])?;
    repo.git(&["commit", "-q", "-m", "submodule"])?;
    repo.head()
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_submodule_change_is_read_whatever_the_repository_ignores() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let one = gitlink_feature(&repo, "1111111111111111111111111111111111111111")?;

    let listed = numstat(repo.path(), "main", &one, TIMEOUT)?;
    let hashed = patch_id(repo.path(), "main", &one, TIMEOUT)?;

    assert_eq!(listed, b"1\t0\tsub\0", "the gitlink is a changed file");
    assert!(hashed.is_some(), "the gitlink is part of the patch");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn an_empty_diff_has_no_patch_id() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let head = repo.head()?;

    assert_eq!(
        patch_id(repo.path(), "main", &head, TIMEOUT)?,
        None,
        "nothing to hash"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_binary_change_changes_the_patch_id() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    repo.git(&["checkout", "-q", "-b", "feature"])?;
    std::fs::write(repo.path().join("logo.bin"), [0_u8, 1, 2])?;
    let one = repo.commit("binary")?;
    let first = patch_id(repo.path(), "main", &one, TIMEOUT)?;
    std::fs::write(repo.path().join("logo.bin"), [0_u8, 1, 3])?;
    repo.git(&["commit", "-q", "-a", "--amend", "-m", "binary"])?;
    let amended = repo.head()?;

    assert_ne!(
        first,
        patch_id(repo.path(), "main", &amended, TIMEOUT)?,
        "binary content is hashed (--binary), not only `Binary files differ`"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_diff_from_a_base_that_does_not_exist_is_an_error() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let head = repo.head()?;

    let result = patch_id(repo.path(), "no-such-base", &head, TIMEOUT);

    assert!(
        matches!(result, Err(DiffError::Failed { what: "diff", .. })),
        "{result:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn commits_resolve_to_full_ids_and_the_default_base_is_found() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let head = repo.head()?;
    let short = head.get(..7).ok_or("a full id")?;

    assert_eq!(
        (
            resolve_commit(repo.path(), short, TIMEOUT)?,
            resolve_commit(repo.path(), "no-such-ref", TIMEOUT)?,
            default_base(repo.path(), TIMEOUT)?,
        ),
        (Some(head.clone()), None, Some("main")),
        "origin/main is absent, so main is the base"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn numstat_lists_each_changed_file_of_the_pull_request() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let head = repo.feature()?;

    let output = numstat(repo.path(), "main", &head, TIMEOUT)?;

    assert_eq!(output, b"3\t0\tsrc/lib.rs\0", "one file, three lines added");
    Ok(())
}

#[test]
fn a_budget_gives_each_call_at_most_its_limit_and_refuses_once_spent() {
    let started = std::time::Instant::now();
    let roomy = Budget::within(started, Duration::from_secs(60), Duration::from_secs(1));
    let spent = Budget::within(started, Duration::ZERO, Duration::from_secs(1));

    assert_eq!(
        roomy.next().ok(),
        Some(Duration::from_secs(1)),
        "the per-call limit"
    );
    assert!(
        matches!(spent.next(), Err(DiffError::OutOfTime { .. })),
        "nothing left"
    );
    assert_eq!(
        Budget::per_call(TIMEOUT).next().ok(),
        Some(TIMEOUT),
        "no deadline"
    );
}

#[test]
fn a_budget_caps_a_wait_at_what_is_left_of_it() {
    let started = std::time::Instant::now();
    let roomy = Budget::within(started, Duration::from_secs(60), Duration::from_secs(1));
    let spent = Budget::within(started, Duration::ZERO, Duration::from_secs(1));
    let lock = Duration::from_millis(500);

    assert_eq!(
        (
            roomy.cap(lock),
            spent.cap(lock),
            Budget::per_call(TIMEOUT).cap(lock)
        ),
        (lock, Duration::ZERO, lock),
        "a ledger lock wait never outlasts the hook's budget"
    );
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_base_is_an_ancestor_of_its_feature_but_not_of_a_sibling_branch() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let base = repo.head()?;
    let feature = repo.feature()?;
    repo.git(&["checkout", "-q", "main"])?;
    repo.write("docs/other.md", "elsewhere\n")?;
    let sibling = repo.commit("sibling")?;

    let answers = (
        is_ancestor(repo.path(), &base, &feature, TIMEOUT)?,
        is_ancestor(repo.path(), &feature, &feature, TIMEOUT)?,
        is_ancestor(repo.path(), &sibling, &feature, TIMEOUT)?,
    );

    assert_eq!(
        answers,
        (true, true, false),
        "the base and the head itself are ancestors; a commit off to the side is not"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn the_diff_text_is_the_change_from_the_base_and_none_over_the_cap() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    let feature = repo.feature()?;

    let limits = |max_bytes| TextLimits {
        timeout: TIMEOUT,
        max_bytes,
    };

    let text = diff_text(repo.path(), "main", &feature, limits(64 * 1024))?;
    let capped = diff_text(repo.path(), "main", &feature, limits(10))?;

    let text = String::from_utf8(text.ok_or("within the cap")?)?;
    assert!(
        text.starts_with("diff --git a/src/lib.rs b/src/lib.rs\n") && text.contains("+    42\n"),
        "the feature's change, a/ and b/ prefixes: {text}"
    );
    assert_eq!(capped, None, "over the cap: no partial diff");
    Ok(())
}
