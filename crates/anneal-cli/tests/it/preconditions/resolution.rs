//! A path is judged where the OS will write it (#80): the existing part resolved before `..`
//! is taken, and a dangling symlink followed to its target.

use super::{Repo, TestResult, assert_allowed, denial};

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_path_through_a_missing_directory_and_dotdot_is_judged_where_it_lands() -> TestResult {
    let repo = Repo::diagnosing()?;

    let into_source = repo.write_to("tmp/missing/../../src/new.rs")?;
    let into_scratch = repo.write_to("src/missing/../../tmp/new.rs")?;

    let reason = denial(&into_source)?;
    assert!(reason.starts_with("ERROR diagnose mode: "), "{reason}");
    assert_allowed(&into_scratch, "lands in tmp/");
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_symlink_followed_by_dotdot_is_judged_where_the_os_writes() -> TestResult {
    let repo = Repo::diagnosing()?;
    std::fs::create_dir_all(repo.path().join("src/deep"))?;
    std::fs::create_dir_all(repo.path().join("tmp"))?;
    std::os::unix::fs::symlink("../src/deep", repo.path().join("tmp/link"))?;

    let reason = denial(&repo.write_to("tmp/link/../app.py")?)?;

    assert!(
        reason.starts_with("ERROR diagnose mode: "),
        "tmp/link/.. is src/: {reason}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_write_through_a_dangling_symlink_is_judged_at_its_target() -> TestResult {
    let repo = Repo::diagnosing()?;
    std::fs::create_dir_all(repo.path().join("tmp"))?;
    std::os::unix::fs::symlink("../src/new.py", repo.path().join("tmp/dangling"))?;

    let reason = denial(&repo.write_to("tmp/dangling")?)?;

    assert!(
        reason.starts_with("ERROR diagnose mode: "),
        "the write creates src/new.py: {reason}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_symlink_into_another_gated_repository_is_judged_by_that_repository() -> TestResult {
    let gated = Repo::diagnosing()?;
    let plain = Repo::new()?;
    std::fs::create_dir_all(gated.path().join("src/deep"))?;
    std::os::unix::fs::symlink(gated.path().join("src/deep"), plain.path().join("link"))?;

    let reason = denial(&plain.write_to("link/../main.rs")?)?;

    assert!(
        reason.starts_with("ERROR diagnose mode: "),
        "the write lands in the gated repository's src/: {reason}"
    );
    Ok(())
}
