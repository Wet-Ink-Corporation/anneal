use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, SystemTimeError};

use super::{STATUSLINE_HINT, statusline_hint};
use crate::context::usage::WindowSource;
use crate::context::{self, Saved, Snapshot, Source};
use crate::decide::clock::Clock;

/// A wall clock stopped at `0` milliseconds since the epoch.
struct At(u64);

impl Clock for At {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        Ok(self.0)
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// The moment the hints below are asked at.
const NOW_MS: u64 = 1_760_000_000_000;

/// Ten minutes, in milliseconds.
const TEN_MINUTES_MS: u64 = 10 * 60 * 1000;

/// The hint the bootstrap shows with the anneal home `home` at [`NOW_MS`], if any.
fn hint_in(home: &Path) -> Option<String> {
    let lookup = |var: &str| (var == "ANNEAL_HOME").then(|| OsString::from(home));
    let (hint, note) = statusline_hint(lookup, &At(NOW_MS));
    assert_eq!(note, None, "the hint's marker is recorded");
    hint
}

/// Saves a mod snapshot for session `s1`, written `age_ms` before [`NOW_MS`].
fn mod_snapshot_aged(home: &Path, age_ms: u64) {
    let reading = Snapshot {
        used_tokens: Some(1_000.0),
        window: 200_000,
        window_source: WindowSource::Model,
    };
    let saved = Saved::stamped(reading, Source::Mod, NOW_MS - age_ms);
    context::save(home, "s1", saved).expect("saves");
}

#[test]
fn with_no_snapshot_the_hint_is_shown_once() {
    let home = tempfile::tempdir().expect("a temporary directory");

    let first = hint_in(home.path());
    let second = hint_in(home.path());

    assert_eq!(first, Some(format!("{STATUSLINE_HINT}\n")), "the hint");
    assert_eq!(second, None, "once per anneal home");
}

#[test]
fn a_mod_snapshot_under_ten_minutes_old_suppresses_the_hint() {
    let home = tempfile::tempdir().expect("a temporary directory");
    mod_snapshot_aged(home.path(), TEN_MINUTES_MS - 1);

    let hint = hint_in(home.path());

    assert_eq!(hint, None, "the mod saves usage: no setup hint");
    assert!(
        !context::in_home(home.path(), context::HINT_FILE).exists(),
        "and it is not spent: the hint can still show if the mod stops"
    );
}

#[test]
fn a_mod_snapshot_ten_minutes_old_or_more_does_not_suppress_the_hint() {
    let home = tempfile::tempdir().expect("a temporary directory");
    mod_snapshot_aged(home.path(), TEN_MINUTES_MS);

    let hint = hint_in(home.path());

    assert_eq!(
        hint,
        Some(format!("{STATUSLINE_HINT}\n")),
        "a stale mod snapshot is no evidence the mod runs"
    );
}
