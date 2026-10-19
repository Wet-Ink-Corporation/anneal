use std::path::Path;

use super::sessions::SessionFile;
use super::usage::WindowSource;
use super::{
    MOD_USAGE_FILE, Saved, Snapshot, Source, USAGE_FILE, USAGE_KIND, freshest, in_home,
    mod_written_within, save,
};

/// A reading of `used` tokens of a 200,000-token window.
const fn reading(used: f64) -> Snapshot {
    Snapshot {
        used_tokens: Some(used),
        window: 200_000,
        window_source: WindowSource::Model,
    }
}

/// Saves `reading` for session `s1` as `source`, written at `at_ms`.
fn store(home: &Path, source: Source, reading: Snapshot, at_ms: u64) {
    save(home, "s1", Saved::stamped(reading, source, at_ms)).expect("saves");
}

/// Saves `used` for session `s1` as `source`, written at `at_ms`.
fn saved_by(home: &Path, source: Source, used: f64, at_ms: u64) {
    store(home, source, reading(used), at_ms);
}

#[test]
fn a_snapshot_saved_before_sources_reads_as_the_status_line_s_unstamped() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let path = in_home(home.path(), USAGE_FILE);
    let text = r#"{"kind":"anneal-context-usage","version":1,"sessions":[{"session":
        "1ce01b1e6d7b6d3fa5b3b0d4b8d1bd7a8b6c94bd4b1b8a3ac23b7d2b4c1f5a37","value":
        {"used_tokens":5.0,"window":200000,"window_source":"default"}}]}"#;
    std::fs::write(&path, text).expect("writes");

    let file = SessionFile::<Saved>::load(&path, USAGE_KIND).expect("an old file loads");
    let entry = file.values().next().copied();

    assert_eq!(
        entry.map(|saved| (saved.source, saved.written_at_ms)),
        Some((Source::Statusline, None)),
        "no source is the status line's, and no time is older than any"
    );
}

#[test]
fn a_saved_snapshot_round_trips_with_its_source_and_time() {
    let home = tempfile::tempdir().expect("a temporary directory");

    store(home.path(), Source::Mod, reading(7.0), 42);
    let file = SessionFile::<Saved>::load(&in_home(home.path(), MOD_USAGE_FILE), USAGE_KIND)
        .expect("loads");

    assert_eq!(
        file.get("s1").copied(),
        Some(Saved {
            reading: reading(7.0),
            source: Source::Mod,
            written_at_ms: Some(42),
        }),
        "the reading, its source and its time, in the mod's own file"
    );
}

#[test]
fn the_reading_alone_still_reads_from_a_stamped_snapshot() {
    let home = tempfile::tempdir().expect("a temporary directory");
    store(home.path(), Source::Statusline, reading(9.0), 1);

    let file = SessionFile::<Snapshot>::load(&in_home(home.path(), USAGE_FILE), USAGE_KIND)
        .expect("a reader of the reading ignores the stamp");

    assert_eq!(file.get("s1").copied(), Some(reading(9.0)), "the reading");
}

#[test]
fn the_fresher_source_wins() {
    let home = tempfile::tempdir().expect("a temporary directory");
    saved_by(home.path(), Source::Mod, 1.0, 100);
    saved_by(home.path(), Source::Statusline, 2.0, 200);

    let newer_status_line = freshest(home.path(), "s1").expect("reads");
    saved_by(home.path(), Source::Mod, 3.0, 300);
    let newer_mod = freshest(home.path(), "s1").expect("reads");

    assert_eq!(
        newer_status_line.map(|saved| (saved.source, saved.reading.used_tokens)),
        Some((Source::Statusline, Some(2.0))),
        "the status line wrote last"
    );
    assert_eq!(
        newer_mod.map(|saved| (saved.source, saved.reading.used_tokens)),
        Some((Source::Mod, Some(3.0))),
        "then the mod did"
    );
}

#[test]
fn a_tie_goes_to_the_mod_and_one_source_alone_is_used() {
    let home = tempfile::tempdir().expect("a temporary directory");
    saved_by(home.path(), Source::Statusline, 2.0, 100);
    let alone = freshest(home.path(), "s1").expect("reads");
    saved_by(home.path(), Source::Mod, 1.0, 100);

    let tied = freshest(home.path(), "s1").expect("reads");

    assert_eq!(
        alone.map(|saved| saved.source),
        Some(Source::Statusline),
        "only the status line"
    );
    assert_eq!(
        tied.map(|saved| saved.source),
        Some(Source::Mod),
        "same time: the mod"
    );
    assert_eq!(
        freshest(home.path(), "s2").expect("reads"),
        None,
        "s2 has none"
    );
}

#[test]
fn a_mod_snapshot_is_recent_only_within_the_window() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let before = mod_written_within(home.path(), 1_000, 600);
    saved_by(home.path(), Source::Mod, 1.0, 1_000);
    saved_by(home.path(), Source::Statusline, 1.0, 1_500);

    let at = |now: u64| mod_written_within(home.path(), now, 600).expect("reads");

    assert!(!before.expect("no file reads"), "no mod snapshot");
    assert!(at(1_599), "599 ms old is within 600 ms");
    assert!(!at(1_600), "600 ms old is not");
}
