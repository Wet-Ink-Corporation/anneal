use std::ffi::OsString;
use std::path::Path;

use super::run;
use crate::context::sessions::SessionFile;
use crate::context::usage::WindowSource;
use crate::context::{self, Snapshot};

/// Runs the status line on `input` with the anneal home `home` and the auto-compact window
/// variable `window`: the exit code's value, stdout and stderr.
fn status_line(input: &str, home: &Path, window: Option<&str>) -> (u8, String, String) {
    let lookup = |var: &str| match var {
        "ANNEAL_HOME" => Some(OsString::from(home)),
        "CLAUDE_CODE_AUTO_COMPACT_WINDOW" => window.map(OsString::from),
        _ => None,
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(input.as_bytes(), lookup, &mut out, &mut err).expect("Vecs accept writes");
    (
        code.value(),
        String::from_utf8(out).expect("UTF-8"),
        String::from_utf8(err).expect("UTF-8"),
    )
}

fn saved(home: &Path, session: &str) -> Option<Snapshot> {
    let path = context::in_home(home, context::USAGE_FILE);
    SessionFile::<Snapshot>::load(&path, context::USAGE_KIND)
        .expect("the usage file loads")
        .get(session)
        .copied()
}

#[test]
fn a_reading_is_shown_and_saved_for_the_session() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let input = r#"{"session_id":"s1","model":{"display_name":"Opus"},
        "context_window":{"used_percentage":60,"context_window_size":200000}}"#;

    let (code, out, err) = status_line(input, home.path(), None);

    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "anneal ctx 120k/200k 60% (hand off soon)\n", ""),
        "one line, nothing on stderr"
    );
    assert_eq!(
        saved(home.path(), "s1"),
        Some(Snapshot {
            used_tokens: Some(120_000.0),
            window: 200_000,
            window_source: WindowSource::Model,
        }),
        "the reading is saved under the session"
    );
}

#[test]
fn the_auto_compact_window_variable_sets_the_window_shown_and_saved() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let input = r#"{"session_id":"s1",
        "context_window":{"used_percentage":10,"context_window_size":1000000}}"#;

    let (_, out, _) = status_line(input, home.path(), Some("400000"));

    assert_eq!(
        out, "anneal ctx 100k/400k 25%\n",
        "10% of 1M tokens against a 400k window"
    );
    assert_eq!(
        saved(home.path(), "s1").map(|snapshot| snapshot.window_source),
        Some(WindowSource::Env),
        "the window's source is saved"
    );
}

#[test]
fn no_percentage_yet_shows_a_dash_and_saves_no_tokens() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let input = r#"{"session_id":"s1","context_window":{"used_percentage":null}}"#;

    let (code, out, _) = status_line(input, home.path(), None);

    assert_eq!(
        (code, out.as_str()),
        (0, "anneal ctx -/200k\n"),
        "nothing to show yet"
    );
    assert_eq!(
        saved(home.path(), "s1").map(|snapshot| snapshot.used_tokens),
        Some(None),
        "the session is recorded, with no tokens"
    );
}

#[test]
fn an_unreadable_input_still_prints_a_line_and_one_error_line() {
    let home = tempfile::tempdir().expect("a temporary directory");

    let (code, out, err) = status_line("not json", home.path(), None);

    assert_eq!(
        (code, out.as_str()),
        (0, "anneal ctx - (input unreadable)\n"),
        "fails open with a line"
    );
    assert!(
        err.starts_with("ERROR the status-line input is not JSON") && err.lines().count() == 1,
        "one ERROR line: {err}"
    );
}

#[test]
fn a_usage_file_that_cannot_be_saved_is_said_in_the_line() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let path = context::in_home(home.path(), context::USAGE_FILE);
    std::fs::create_dir(&path).expect("a directory where the file goes");
    let input = r#"{"session_id":"s1","context_window":{"used_percentage":1}}"#;

    let (code, out, err) = status_line(input, home.path(), None);

    assert_eq!(
        (code, out.as_str()),
        (0, "anneal ctx 2k/200k 1% (usage not saved)\n"),
        "the line says the monitor will not see it"
    );
    assert_eq!(err.lines().count(), 1, "one ERROR line: {err}");
}
