use std::path::PathBuf;

use super::BriefError;
use crate::exit_codes;
use crate::pr_diff::DiffError;

fn failed(what: &'static str) -> DiffError {
    DiffError::Failed {
        what,
        stderr: "fatal: bad".to_owned(),
    }
}

#[test]
fn resolving_the_range_and_reading_the_diff_fail_with_their_own_messages() {
    let range = BriefError::Range(failed("rev-parse"));
    let diff = BriefError::Diff(failed("diff"));

    assert_eq!(
        (range.to_string(), diff.to_string()),
        (
            "resolving the range".to_owned(),
            "reading the diff".to_owned()
        ),
        "each git step names itself"
    );
    assert_eq!(
        (range.code(), diff.code()),
        (exit_codes::INTERNAL, exit_codes::INTERNAL),
        "git failing is internal"
    );
}

#[test]
fn a_briefs_directory_resolving_elsewhere_is_internal_and_names_where() {
    let error = BriefError::NotOwnDirectory {
        resolved: PathBuf::from("/elsewhere/briefs"),
    };

    assert_eq!(
        error.to_string(),
        ".anneal/briefs resolves to /elsewhere/briefs, not this worktree's own .anneal/briefs; \
         make it a plain directory",
        "where it leads, and the fix"
    );
    assert_eq!(
        error.code(),
        exit_codes::INTERNAL,
        "nothing the arguments can fix"
    );
}
