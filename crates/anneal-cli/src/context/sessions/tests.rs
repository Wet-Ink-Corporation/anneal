use super::{MAX_SESSIONS, SessionFile, SessionsError};
use crate::baseline::BaselineError;

const KIND: &str = "anneal-test-sessions";

#[test]
fn a_missing_file_is_no_sessions() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let file = SessionFile::<u32>::load(&dir.path().join("none.json"), KIND).expect("loads");

    assert_eq!(file.get("s1"), None, "nothing recorded");
}

#[test]
fn a_saved_value_loads_back_by_session() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("state.json");
    let mut file = SessionFile::<u32>::load(&path, KIND).expect("loads");
    file.set("s1", 7);
    file.set("s2", 9);
    file.save(&path).expect("saves");

    let loaded = SessionFile::<u32>::load(&path, KIND).expect("loads again");

    assert_eq!(
        (loaded.get("s1"), loaded.get("s2")),
        (Some(&7), Some(&9)),
        "each session's value"
    );
    let text = std::fs::read_to_string(&path).expect("readable");
    assert!(!text.contains("\"s1\""), "session ids are hashed: {text}");
}

#[test]
fn setting_keeps_the_newest_sessions_up_to_the_cap() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut file = SessionFile::<u32>::load(&dir.path().join("x.json"), KIND).expect("loads");
    for n in 0..=MAX_SESSIONS {
        file.set(&format!("s{n}"), 1);
    }
    file.set("s1", 2);

    assert_eq!(file.get("s0"), None, "the oldest was dropped");
    assert_eq!(file.get("s1"), Some(&2), "re-set, so newest and updated");
    assert_eq!(file.len(), MAX_SESSIONS, "never more than the cap");
}

#[test]
fn a_file_of_another_kind_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("state.json");
    let mut file = SessionFile::<u32>::load(&path, "anneal-other").expect("loads");
    file.set("s1", 1);
    file.save(&path).expect("saves");

    let loaded = SessionFile::<u32>::load(&path, KIND);

    assert!(
        matches!(&loaded, Err(SessionsError::Foreign { kind, version: 1, .. }) if kind == "anneal-other"),
        "not this file's kind: {loaded:?}"
    );
}

#[test]
fn a_directory_at_the_path_is_refused_without_being_opened() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("state.json");
    std::fs::create_dir(&path).expect("a directory where the file goes");

    let loaded = SessionFile::<u32>::load(&path, KIND);

    assert!(
        matches!(&loaded, Err(SessionsError::File(BaselineError::NotAFile { path: refused })) if *refused == path),
        "a directory is not a state file: {loaded:?}"
    );
}
