//! `read_fixtures` on disk: which entries it reads, which it skips, and the faults it reports.

use std::error::Error;
use std::fs;
use std::path::Path;

use crate::eval::routing::{Fault, Problem, judge, read_fixtures};
use crate::lint::skills::discover::MAX_READ_BYTES;
use crate::lint::skills::listing::Listing;

type TestResult = Result<(), Box<dyn Error>>;

fn listing(name: &str) -> Listing {
    Listing {
        name: name.to_owned(),
        text: name.to_owned(),
        model_invoked: true,
    }
}

fn shown(dir: &Path, name: &str) -> String {
    dir.join(name).display().to_string()
}

#[test]
fn unreadable_fixtures_are_faults_and_other_entries_are_skipped() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("spec.yaml"), [0xff, 0xfe, b'\n'])?;
    fs::File::create(dir.path().join("diagnose.yaml"))?.set_len(MAX_READ_BYTES + 1)?;
    fs::write(dir.path().join("README.md"), "# notes\n")?;
    fs::write(
        dir.path().join("plan.yml"),
        "positives:\n  - break into tasks\n",
    )?;
    fs::create_dir(dir.path().join("plan.yaml"))?;

    let sources = read_fixtures(dir.path())?;
    let skills = vec![
        Ok(listing("diagnose")),
        Ok(listing("plan")),
        Ok(listing("spec")),
    ];
    let outcome = judge(skills, sources);

    let at = |name: &str, fault| Problem::Fixture {
        file: shown(dir.path(), name),
        line: 0,
        fault,
    };
    assert_eq!(
        outcome.problems,
        [
            at(
                "diagnose.yaml",
                Fault::TooLarge {
                    size: MAX_READ_BYTES + 1
                }
            ),
            at("spec.yaml", Fault::NotUtf8),
            Problem::MissingFixture {
                skill: "plan".to_owned()
            },
        ],
        "README.md, plan.yml and the plan.yaml directory are not fixtures"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn a_yaml_symlink_to_a_file_is_read_and_one_to_a_directory_is_skipped() -> TestResult {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir()?;
    let fixtures = dir.path().join("routing");
    fs::create_dir(&fixtures)?;
    let target = dir.path().join("plan-source.txt");
    fs::write(&target, "positives:\n  - break into tasks\n")?;
    symlink(&target, fixtures.join("plan.yaml"))?;
    symlink(dir.path(), fixtures.join("spec.yaml"))?;

    let sources = read_fixtures(&fixtures)?;

    let read: Vec<(Option<&str>, Option<usize>)> = sources
        .iter()
        .map(|s| {
            let positives = s.parsed.as_ref().ok().map(|f| f.positives.len());
            (s.skill.as_deref(), positives)
        })
        .collect();
    assert_eq!(
        read,
        [(Some("plan"), Some(1))],
        "the file link is read through; the directory link is skipped"
    );
    Ok(())
}
