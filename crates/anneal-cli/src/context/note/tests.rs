use std::error::Error;

use super::{MAX_NOTE_BYTES, ReadError, is_recorded, notes_start, read, record};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn a_missing_note_reads_as_none() -> TestResult {
    let dir = tempfile::tempdir()?;

    assert_eq!(read(&dir.path().join("absent"))?, None);
    Ok(())
}

#[test]
fn a_note_over_the_cap_is_refused_by_its_length() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("note");
    let size = usize::try_from(MAX_NOTE_BYTES)?.saturating_add(1);
    std::fs::write(&path, vec![b'x'; size])?;

    let refused = read(&path);

    assert!(
        matches!(refused, Err(ReadError::TooLarge { .. })),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn a_directory_is_not_a_note() -> TestResult {
    let dir = tempfile::tempdir()?;

    let refused = read(dir.path());

    assert!(
        matches!(refused, Err(ReadError::NotAFile { .. })),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn the_notes_start_only_at_a_heading_at_a_line_start() {
    assert_eq!(notes_start("## Notes\n"), Some(0));
    assert_eq!(notes_start("a\n## Notes\n"), Some(2));
    assert_eq!(notes_start("a ## Notes\n"), None);
}

#[test]
fn only_the_recorded_head_is_recorded_whatever_the_notes_say() -> TestResult {
    let home = tempfile::tempdir()?;
    let written = "# Continue here\n\n- Task: T2\n\n## Notes\n\n- one\n";

    record(home.path(), "/repo", written)?;

    assert!(is_recorded(home.path(), "/repo", written)?, "as written");
    assert!(
        is_recorded(home.path(), "/repo", &format!("{written}- two\n"))?,
        "notes added"
    );
    assert!(
        !is_recorded(home.path(), "/repo", &written.replace("T2", "T3"))?,
        "head changed"
    );
    assert!(
        !is_recorded(home.path(), "/other", written)?,
        "another worktree"
    );
    Ok(())
}
