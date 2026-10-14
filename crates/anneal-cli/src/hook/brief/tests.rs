use std::fs;

use super::{BriefReadError, MAX_BRIEF_BYTES, read_brief};

#[test]
fn a_brief_within_the_cap_is_read_whole() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("36.md");
    fs::write(&path, "## Goal\nShip it.\n").expect("written");

    assert_eq!(
        read_brief(&path).expect("read"),
        "## Goal\nShip it.\n",
        "the whole file"
    );
}

#[test]
fn a_brief_over_the_cap_is_refused_rather_than_cut() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("36.md");
    let size = usize::try_from(MAX_BRIEF_BYTES).expect("fits") + 1;
    fs::write(&path, "x".repeat(size)).expect("written");

    assert!(
        matches!(read_brief(&path), Err(BriefReadError::TooLarge)),
        "one byte over"
    );
}

#[test]
fn a_directory_named_as_the_brief_is_not_a_file() {
    let dir = tempfile::tempdir().expect("a directory");

    assert!(
        matches!(read_brief(dir.path()), Err(BriefReadError::NotAFile)),
        "a directory is never opened as a brief"
    );
}
