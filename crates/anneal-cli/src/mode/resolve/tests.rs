use std::error::Error;
use std::fs;
use std::path::PathBuf;

use super::resolve;

type TestResult = Result<(), Box<dyn Error>>;

/// A temporary directory and its resolved path (macOS's `/var` is `/private/var`).
fn scratch() -> Result<(tempfile::TempDir, PathBuf), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let real = fs::canonicalize(dir.path())?;
    Ok((dir, real))
}

#[test]
fn a_missing_tail_is_taken_lexically() -> TestResult {
    let (dir, real) = scratch()?;

    let resolved = resolve(
        &dir.path()
            .join("a")
            .join(".")
            .join("b")
            .join("..")
            .join("c"),
    )?;

    assert_eq!(resolved, real.join("a").join("c"));
    Ok(())
}

#[test]
fn dotdot_after_an_existing_directory_climbs_to_its_parent() -> TestResult {
    let (dir, real) = scratch()?;
    fs::create_dir_all(dir.path().join("src").join("deep"))?;

    let resolved = resolve(&dir.path().join("src").join("deep").join("..").join("x.rs"))?;

    assert_eq!(resolved, real.join("src").join("x.rs"));
    Ok(())
}

#[test]
fn an_existing_file_resolves_to_itself() -> TestResult {
    let (dir, real) = scratch()?;
    fs::write(dir.path().join("x.rs"), "")?;

    let resolved = resolve(&dir.path().join("x.rs"))?;

    assert_eq!(resolved, real.join("x.rs"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn dotdot_after_a_symlink_climbs_from_its_target() -> TestResult {
    let (dir, real) = scratch()?;
    fs::create_dir_all(dir.path().join("src").join("deep"))?;
    fs::create_dir_all(dir.path().join("tmp"))?;
    std::os::unix::fs::symlink("../src/deep", dir.path().join("tmp").join("link"))?;

    let resolved = resolve(&dir.path().join("tmp/link/../app.py"))?;

    assert_eq!(resolved, real.join("src").join("app.py"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_followed_to_its_target() -> TestResult {
    let (dir, real) = scratch()?;
    fs::create_dir_all(dir.path().join("tmp"))?;
    std::os::unix::fs::symlink("../src/new.py", dir.path().join("tmp").join("dangling"))?;

    let resolved = resolve(&dir.path().join("tmp").join("dangling"))?;

    assert_eq!(resolved, real.join("src").join("new.py"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_to_an_absolute_target_starts_again_at_the_root() -> TestResult {
    let (dir, real) = scratch()?;
    std::os::unix::fs::symlink(dir.path().join("src/new.py"), dir.path().join("dangling"))?;

    let resolved = resolve(&dir.path().join("dangling"))?;

    assert_eq!(resolved, real.join("src").join("new.py"));
    Ok(())
}

/// A chain of `links` dangling symlinks `l0 -> l1 -> ... -> l<links-1> -> missing`.
#[cfg(unix)]
fn chain(dir: &std::path::Path, links: usize) -> TestResult {
    for index in 0..links {
        let next = if index + 1 == links {
            "missing".to_owned()
        } else {
            format!("l{}", index + 1)
        };
        std::os::unix::fs::symlink(next, dir.join(format!("l{index}")))?;
    }
    Ok(())
}

/// Whether `error` is [`super::TooManyLinks`].
#[cfg(unix)]
fn is_too_many_links(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(<dyn Error + Send + Sync>::is::<super::TooManyLinks>)
}

#[cfg(unix)]
#[test]
fn forty_dangling_symlinks_are_followed_and_a_forty_first_is_an_error() -> TestResult {
    let (_dir, real) = scratch()?;
    chain(&real, 41)?;

    // From the canonical directory: on macOS the temporary directory's own path holds a link
    // (`/var` to `/private/var`), which would count against the 40.
    let forty = resolve(&real.join("l1"))?;
    let error = resolve(&real.join("l0")).expect_err("41 links are too many");

    assert_eq!(forty, real.join("missing"));
    assert!(is_too_many_links(&error), "{error:?}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_is_an_error() -> TestResult {
    let (dir, _real) = scratch()?;
    std::os::unix::fs::symlink("b", dir.path().join("a"))?;
    std::os::unix::fs::symlink("a", dir.path().join("b"))?;

    let error = resolve(&dir.path().join("a").join("x.rs")).expect_err("a loop never ends");

    assert!(is_too_many_links(&error), "{error:?}");
    Ok(())
}
