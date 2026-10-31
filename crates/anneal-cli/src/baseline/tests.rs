use super::{
    Baseline, BaselineError, create_temporary, load_capped, temporary_path,
    write_snapshot_json_if_changed,
};
use crate::envelope::Finding;
use std::io::ErrorKind;
use std::path::Path;

const SNAPSHOT: &str = r#"{"version": 1, "keys": ["a"]}"#;

fn finding(key: &str) -> Finding {
    Finding::new(key, format!("message for {key}"))
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn only_findings_absent_from_the_snapshot_are_new() {
    let baseline = Baseline::from_findings(&[finding("a"), finding("b")]);
    let later = [finding("b"), finding("c"), finding("a"), finding("d")];

    let new: Vec<&str> = baseline
        .new_findings(&later)
        .map(|f| f.key.as_str())
        .collect();

    assert_eq!(new, ["c", "d"], "a and b were already there");
}

#[test]
fn a_saved_snapshot_loads_back_equal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("nested/baseline.json");
    let baseline = Baseline::from_findings(&[finding("z"), finding("a")]);

    baseline.save(&path).expect("save");
    let loaded = Baseline::load(&path).expect("load");

    assert_eq!(loaded, Some(baseline), "round trip keeps the keys");
}

#[test]
fn a_missing_snapshot_loads_as_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    let loaded = Baseline::load(&dir.path().join("absent.json")).expect("load");
    assert_eq!(loaded, None, "no snapshot taken yet");
}

#[test]
fn an_unknown_version_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, r#"{"version": 2, "keys": []}"#).expect("write");

    let error = Baseline::load(&path).expect_err("version 2 is unknown");

    assert!(
        matches!(error, BaselineError::Version { found: 2, .. }),
        "got {error:?}"
    );
}

#[test]
fn a_newer_version_with_a_changed_shape_is_a_version_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, r#"{"version": 2, "entries": [{"key": "a"}]}"#).expect("write");

    let error = Baseline::load(&path).expect_err("version 2 is unknown");

    assert!(
        matches!(error, BaselineError::Version { found: 2, .. }),
        "got {error:?}"
    );
}

#[test]
fn garbage_is_a_format_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, "not json").expect("write");

    let error = Baseline::load(&path).expect_err("not a snapshot");

    assert!(
        matches!(error, BaselineError::Format { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_snapshot_over_the_cap_is_too_large() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, SNAPSHOT).expect("write");
    let cap = u64::try_from(SNAPSHOT.len()).expect("small") - 1;

    let error = load_capped(&path, cap).expect_err("one byte over the cap");

    assert!(
        matches!(error, BaselineError::TooLarge { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_snapshot_exactly_at_the_cap_loads() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, SNAPSHOT).expect("write");
    let cap = u64::try_from(SNAPSHOT.len()).expect("small");

    let loaded = load_capped(&path, cap).expect("at the cap is allowed");

    assert_eq!(
        loaded,
        Some(Baseline::from_findings(&[finding("a")])),
        "the snapshot loads in full"
    );
}

#[test]
fn temporary_paths_are_distinct_siblings_tagged_with_the_pid() {
    let target = Path::new("dir").join("baseline.json");

    let first = temporary_path(&target);
    let second = temporary_path(&target);

    assert_ne!(first, second, "each writer gets its own name");
    for temporary in [&first, &second] {
        assert_eq!(
            temporary.parent(),
            target.parent(),
            "same directory: {temporary:?}"
        );
        let name = temporary
            .file_name()
            .expect("a file name")
            .to_string_lossy();
        let prefix = format!("baseline.json.{}.", std::process::id());
        assert!(
            name.starts_with(&prefix) && name.ends_with(".tmp"),
            "named <file>.<pid>.<n>.tmp: {name}"
        );
    }
}

#[test]
fn a_taken_temporary_name_is_skipped_for_the_next() {
    let dir = tempfile::tempdir().expect("temp dir");
    let taken = dir.path().join("first.tmp");
    let free = dir.path().join("second.tmp");
    std::fs::write(&taken, "left by a crashed writer").expect("pre-create");
    let mut names = [taken.clone(), free.clone()].into_iter();

    let (chosen, _file) =
        create_temporary(|| names.next().expect("only two names asked for")).expect("creates");

    assert_eq!(chosen, free, "the second name was used");
    assert_eq!(
        std::fs::read_to_string(&taken).expect("still there"),
        "left by a crashed writer",
        "the taken file was not touched"
    );
}

#[test]
fn when_every_temporary_name_is_taken_the_error_is_already_exists() {
    let dir = tempfile::tempdir().expect("temp dir");
    let taken = dir.path().join("stale.tmp");
    std::fs::write(&taken, "stale").expect("pre-create");

    let error = create_temporary(|| taken.clone()).expect_err("every name is taken");

    assert_eq!(error.kind(), ErrorKind::AlreadyExists, "got {error:?}");
    assert!(
        error.to_string().contains("stale temporary files"),
        "the message points at stale files: {error}"
    );
}

#[test]
fn saving_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("baseline.json");

    Baseline::from_findings(&[finding("a")])
        .save(&path)
        .expect("first save");
    Baseline::from_findings(&[finding("b")])
        .save(&path)
        .expect("overwrite");

    assert_eq!(
        file_names(dir.path()),
        ["baseline.json"],
        "only the snapshot remains"
    );
}

#[test]
fn a_failed_save_removes_its_temporary_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    // A non-empty directory where the snapshot should go makes the rename fail.
    let path = dir.path().join("baseline.json");
    std::fs::create_dir(&path).expect("mkdir");
    std::fs::write(path.join("occupant"), "x").expect("write");

    let error = Baseline::from_findings(&[finding("a")])
        .save(&path)
        .expect_err("cannot rename over a directory");

    assert!(
        matches!(error, BaselineError::Io { op: "writing", .. }),
        "got {error:?}"
    );
    assert_eq!(file_names(dir.path()), ["baseline.json"], "no temp left");
}

#[test]
fn a_file_longer_than_the_new_json_is_replaced_by_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("proposals.json");
    let longer = format!("{{\"keys\": [{}]}}\n", "\"a\", ".repeat(100) + "\"a\"");
    std::fs::write(&path, &longer).expect("seed");

    let written = write_snapshot_json_if_changed(&path, &serde_json::json!({"keys": []}));

    assert!(written.is_ok(), "{written:?}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        "{\n  \"keys\": []\n}\n"
    );
    assert_eq!(file_names(dir.path()), ["proposals.json"], "no temp left");
}

#[test]
fn a_file_holding_the_same_json_is_left_as_it_is() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("proposals.json");
    let value = serde_json::json!({"keys": ["a"]});
    write_snapshot_json_if_changed(&path, &value).expect("first write");
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(old))
        .expect("age the file");

    write_snapshot_json_if_changed(&path, &value).expect("second write");

    let modified = std::fs::metadata(&path).and_then(|meta| meta.modified());
    assert_eq!(modified.expect("mtime"), old, "not rewritten");
}
