use std::fs;
use std::path::{Path, PathBuf};

use super::{FixtureName, RepoRoot, locate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A repository at a fresh temporary directory, its suites at `evals/uplift/`.
struct Repo {
    root: tempfile::TempDir,
}

impl Repo {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("evals/uplift/fixtures"))?;
        Ok(Self { root })
    }

    fn suites(&self) -> PathBuf {
        self.root.path().join("evals/uplift")
    }

    /// The fixture directory `name`, made.
    fn fixture(&self, name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dir = self.suites().join("fixtures").join(name);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn locate(&self, name: &str) -> Result<String, String> {
        let suites = self.suites();
        let root = RepoRoot::new(&suites);
        locate(&suites, &name_of(name), &root)
    }
}

fn name_of(name: &str) -> FixtureName {
    FixtureName::try_from(name.to_owned()).expect("a fixture name")
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path)
        .expect("absolute")
        .to_str()
        .expect("UTF-8")
        .to_owned()
}

#[test]
fn a_fixture_holding_a_file_is_located_at_its_absolute_path() -> TestResult {
    let repo = Repo::new()?;
    let dir = repo.fixture("crate")?;
    fs::create_dir_all(dir.join("src"))?;
    fs::write(dir.join("src/lib.rs"), "")?;

    assert_eq!(repo.locate("crate"), Ok(absolute(&dir)));
    Ok(())
}

#[test]
fn a_missing_fixture_is_refused() -> TestResult {
    let repo = Repo::new()?;

    assert_eq!(
        repo.locate("absent"),
        Err("no fixture directory fixtures/absent".to_owned())
    );
    Ok(())
}

#[test]
fn a_fixture_with_no_file_is_refused() -> TestResult {
    let repo = Repo::new()?;
    let dir = repo.fixture("hollow")?;
    fs::create_dir_all(dir.join("a/b"))?;

    assert_eq!(
        repo.locate("hollow"),
        Err("fixtures/hollow holds no file".to_owned())
    );
    Ok(())
}

#[test]
fn a_fixture_that_is_a_file_is_refused() -> TestResult {
    let repo = Repo::new()?;
    fs::write(repo.suites().join("fixtures/flat"), "x")?;

    assert_eq!(
        repo.locate("flat"),
        Err("fixtures/flat is not a directory".to_owned())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess to find the repository root")]
fn a_symlinked_subdirectory_inside_the_repository_counts_as_content() -> TestResult {
    let repo = Repo::new()?;
    fs::create_dir_all(repo.root.path().join("docs"))?;
    fs::write(repo.root.path().join("docs/guide.md"), "guide")?;
    let dir = repo.fixture("docs-only")?;
    std::os::unix::fs::symlink("../../../../docs", dir.join("docs"))?;

    assert_eq!(repo.locate("docs-only"), Ok(absolute(&dir)));
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess to find the repository root")]
fn a_symlink_to_a_file_inside_the_repository_counts_as_content() -> TestResult {
    let repo = Repo::new()?;
    fs::write(repo.root.path().join("README.md"), "readme")?;
    let dir = repo.fixture("readme")?;
    std::os::unix::fs::symlink("../../../../README.md", dir.join("README.md"))?;

    assert_eq!(repo.locate("readme"), Ok(absolute(&dir)));
    Ok(())
}

#[test]
#[cfg(unix)]
fn a_dangling_symlink_is_refused() -> TestResult {
    let repo = Repo::new()?;
    let dir = repo.fixture("dangling")?;
    fs::write(dir.join("a.txt"), "a")?;
    std::os::unix::fs::symlink("no-such-file", dir.join("gone"))?;

    assert_eq!(
        repo.locate("dangling"),
        Err("fixtures/dangling/gone is a dangling symlink".to_owned())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess to find the repository root")]
fn a_symlink_leading_out_of_the_repository_is_refused() -> TestResult {
    let repo = Repo::new()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("secret"), "s")?;
    let dir = repo.fixture("escape")?;
    std::os::unix::fs::symlink(outside.path().join("secret"), dir.join("secret"))?;

    assert_eq!(
        repo.locate("escape"),
        Err("fixtures/escape/secret leads out of the repository".to_owned())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess to find the repository root")]
fn a_symlink_inside_a_symlinked_subdirectory_is_checked_too() -> TestResult {
    let repo = Repo::new()?;
    let outside = tempfile::tempdir()?;
    fs::create_dir_all(repo.root.path().join("docs"))?;
    std::os::unix::fs::symlink(outside.path(), repo.root.path().join("docs/out"))?;
    let dir = repo.fixture("nested")?;
    std::os::unix::fs::symlink("../../../../docs", dir.join("docs"))?;

    assert_eq!(
        repo.locate("nested"),
        Err("fixtures/nested/docs/out leads out of the repository".to_owned())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn a_symlinked_fixture_directory_is_refused() -> TestResult {
    let repo = Repo::new()?;
    let real = repo.fixture("real")?;
    fs::write(real.join("a.txt"), "a")?;
    std::os::unix::fs::symlink("real", repo.suites().join("fixtures/alias"))?;

    assert_eq!(
        repo.locate("alias"),
        Err("fixtures/alias is a symlink; a fixture is a plain directory".to_owned())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess to find the repository root")]
fn a_symlink_back_into_a_directory_it_is_inside_is_refused() -> TestResult {
    let repo = Repo::new()?;
    let dir = repo.fixture("loop")?;
    fs::create_dir_all(dir.join("a"))?;
    fs::write(dir.join("a/x.txt"), "x")?;
    std::os::unix::fs::symlink("..", dir.join("a/up"))?;

    assert_eq!(
        repo.locate("loop"),
        Err("fixtures/loop/a/up leads back into a directory it is inside".to_owned())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess to find the repository root")]
fn two_symlinks_to_one_directory_are_no_loop() -> TestResult {
    let repo = Repo::new()?;
    fs::create_dir_all(repo.root.path().join("docs"))?;
    fs::write(repo.root.path().join("docs/guide.md"), "guide")?;
    let dir = repo.fixture("twice")?;
    std::os::unix::fs::symlink("../../../../docs", dir.join("one"))?;
    std::os::unix::fs::symlink("../../../../docs", dir.join("two"))?;

    assert_eq!(repo.locate("twice"), Ok(absolute(&dir)));
    Ok(())
}

#[test]
fn a_fixture_over_the_entry_cap_is_refused() -> TestResult {
    let repo = Repo::new()?;
    let dir = repo.fixture("huge")?;
    for n in 0..=10_000 {
        fs::write(dir.join(n.to_string()), "")?;
    }

    assert_eq!(
        repo.locate("huge"),
        Err("fixtures/huge holds more than 10000 entries".to_owned())
    );
    Ok(())
}

/// Locates fixture `name` under `suites`, its root found afresh.
fn locate_in(suites: &Path, name: &str) -> Result<String, String> {
    let root = RepoRoot::new(suites);
    locate(suites, &name_of(name), &root)
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn in_a_git_repository_its_top_level_bounds_the_links_even_under_an_evals_directory() -> TestResult
{
    if !crate::eval::uplift::test_git::available()? {
        return Ok(());
    }
    // A clone under some `evals/` directory, its suites with no `evals` component of their own:
    // git's top level, not the parent of that outer `evals/`, is the repository.
    let tmp = tempfile::tempdir()?;
    let clone = tmp.path().join("evals/clone");
    let suites = clone.join("cases/uplift");
    fs::create_dir_all(&suites)?;
    crate::eval::uplift::test_git::init(&clone)?;
    fs::write(clone.join("README.md"), "readme")?;
    fs::create_dir_all(tmp.path().join("evals/elsewhere"))?;
    fs::write(tmp.path().join("evals/elsewhere/secret"), "s")?;
    let inside = suites.join("fixtures/inside");
    fs::create_dir_all(&inside)?;
    std::os::unix::fs::symlink("../../../../README.md", inside.join("README.md"))?;
    let outside = suites.join("fixtures/outside");
    fs::create_dir_all(&outside)?;
    std::os::unix::fs::symlink("../../../../../elsewhere/secret", outside.join("secret"))?;

    assert_eq!(locate_in(&suites, "inside"), Ok(absolute(&inside)));
    assert_eq!(
        locate_in(&suites, "outside"),
        Err("fixtures/outside/secret leads out of the repository".to_owned())
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn outside_any_repository_a_symlink_is_refused_for_want_of_a_root() -> TestResult {
    if !crate::eval::uplift::test_git::available()? {
        return Ok(());
    }
    let tmp = tempfile::tempdir()?;
    let suites = tmp.path().join("plain/uplift");
    let dir = suites.join("fixtures/linked");
    fs::create_dir_all(&dir)?;
    fs::write(tmp.path().join("plain/README.md"), "readme")?;
    std::os::unix::fs::symlink("../../../README.md", dir.join("README.md"))?;

    assert_eq!(
        locate_in(&suites, "linked"),
        Err(
            "fixtures/linked/README.md is a symlink, and no repository root was found to \
             check it against: the suites are in no git repository"
                .to_owned()
        )
    );
    Ok(())
}

#[test]
fn outside_any_repository_a_fixture_without_symlinks_is_located() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let suites = tmp.path().join("plain/uplift");
    let dir = suites.join("fixtures/plain");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("a.txt"), "a")?;

    assert_eq!(locate_in(&suites, "plain"), Ok(absolute(&dir)));
    Ok(())
}
