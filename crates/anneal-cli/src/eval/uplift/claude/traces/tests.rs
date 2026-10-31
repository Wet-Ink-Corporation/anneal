use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{Trace, keep};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A fresh directory and its resolved path, as `claude` names the paths in it.
fn resolved_dir() -> Result<(TempDir, PathBuf), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = fs::canonicalize(dir.path())?;
    Ok((dir, path))
}

/// The trace kept as `traces/1-with/c-1.jsonl` from `path`.
fn trace(path: &Path) -> Result<Trace, Box<dyn std::error::Error>> {
    Ok(Trace {
        name: "traces/1-with/c-1.jsonl".to_owned(),
        path: path.to_str().ok_or("a UTF-8 path")?.to_owned(),
    })
}

/// The names of the traces `keep` did not keep, whether each reason holds `why`, and
/// whether anything was made below `dir`.
fn findings<'u>(unkept: &'u [super::Unkept], why: &str, dir: &Path) -> (Vec<&'u str>, bool, bool) {
    (
        unkept.iter().map(|u| u.name.as_str()).collect(),
        unkept.iter().all(|u| u.reason.contains(why)),
        dir.join("traces").exists(),
    )
}

/// One finding for the trace, holding its reason, and nothing made.
fn one_finding() -> (Vec<&'static str>, bool, bool) {
    (vec!["traces/1-with/c-1.jsonl"], true, false)
}

#[test]
fn a_trace_named_inside_the_workspace_is_copied() -> TestResult {
    let (_root, root) = resolved_dir()?;
    let dir = tempfile::tempdir()?;
    fs::write(root.join("trace.jsonl"), "{}\n")?;

    let unkept = keep(&[trace(&root.join("trace.jsonl"))?], &root, dir.path());

    assert_eq!(
        (
            unkept,
            fs::read_to_string(dir.path().join("traces/1-with/c-1.jsonl"))?
        ),
        (Vec::new(), "{}\n".to_owned())
    );
    Ok(())
}

#[test]
fn a_trace_named_outside_the_workspace_is_skipped() -> TestResult {
    let (_root, root) = resolved_dir()?;
    let (_elsewhere, elsewhere) = resolved_dir()?;
    let dir = tempfile::tempdir()?;
    fs::write(elsewhere.join("trace.jsonl"), "{}\n")?;

    let unkept = keep(&[trace(&elsewhere.join("trace.jsonl"))?], &root, dir.path());

    assert_eq!(
        (unkept, dir.path().join("traces").exists()),
        (Vec::new(), false)
    );
    Ok(())
}

/// Unix only: a Windows resolved path is verbatim (`\\?\`), where `..` is not resolved.
#[test]
#[cfg(unix)]
fn a_trace_named_inside_that_climbs_out_is_a_finding() -> TestResult {
    let (_parent, parent) = resolved_dir()?;
    let root = parent.join("ae-000000");
    fs::create_dir(&root)?;
    fs::write(parent.join("trace.jsonl"), "{}\n")?;
    let dir = tempfile::tempdir()?;

    let unkept = keep(&[trace(&root.join("../trace.jsonl"))?], &root, dir.path());

    assert_eq!(
        findings(&unkept, "resolves outside the workspace", dir.path()),
        one_finding()
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn a_trace_named_inside_that_links_out_is_a_finding() -> TestResult {
    let (_root, root) = resolved_dir()?;
    let (_elsewhere, elsewhere) = resolved_dir()?;
    fs::write(elsewhere.join("trace.jsonl"), "{}\n")?;
    std::os::unix::fs::symlink(elsewhere.join("trace.jsonl"), root.join("trace.jsonl"))?;
    let dir = tempfile::tempdir()?;

    let unkept = keep(&[trace(&root.join("trace.jsonl"))?], &root, dir.path());

    assert_eq!(
        findings(&unkept, "resolves outside the workspace", dir.path()),
        one_finding()
    );
    Ok(())
}

#[test]
fn a_trace_that_is_not_a_file_is_a_finding_and_nothing_is_made() -> TestResult {
    let (_root, root) = resolved_dir()?;
    fs::create_dir(root.join("trace.jsonl"))?;
    let dir = tempfile::tempdir()?;

    let unkept = keep(&[trace(&root.join("trace.jsonl"))?], &root, dir.path());

    assert_eq!(
        findings(&unkept, "is not a file", dir.path()),
        one_finding()
    );
    Ok(())
}
