//! What the bootstrap adds so a session survives compaction: the `.continue-here` note the
//! `handoff` hook wrote, and a one-time hint while no status line saves context usage.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::baseline;
use crate::context;
use crate::error_chain::reason;
use crate::hook::handoff::CONTINUE_HERE;
use crate::paths::{self, PathError};

/// The most of `.continue-here` injected, in bytes; the handoff hook writes far less (H-13).
pub(crate) const MAX_NOTE_BYTES: u64 = 16 * 1024;

/// The setup hint, shown once per anneal home while no status line saves context usage.
pub(crate) const STATUSLINE_HINT: &str = "anneal: the context monitor is off, because no status \
    line saves context usage (hooks receive none). Set it up once: in ~/.claude/settings.json, \
    make `statusLine` run `anneal statusline` through the plugin's launcher; the anneal README's \
    \"Status line\" section has the exact setting. This hint is shown once.";

/// The `.continue-here` note at the root of `cwd`'s worktree, and a note when it could not be
/// read. Nothing outside a git repository or without the file.
pub(crate) fn continue_here(cwd: &Path, git_timeout: Duration) -> (Option<String>, Option<String>) {
    let unread = |why: String| {
        (
            None,
            Some(format!("anneal could not read {CONTINUE_HERE}: {why}")),
        )
    };
    let root = match paths::worktree_root_within(cwd, git_timeout) {
        Ok(root) => root,
        Err(PathError::NotARepository { .. }) => return (None, None),
        Err(error) => return unread(reason(&error)),
    };
    let path = Path::new(&root).join(CONTINUE_HERE);
    match baseline::read_state_bytes(&path) {
        Ok(None) => (None, None),
        Ok(Some(bytes)) if u64::try_from(bytes.len()).is_ok_and(|len| len <= MAX_NOTE_BYTES) => {
            let mut note = String::from_utf8_lossy(&bytes).into_owned();
            if !note.ends_with('\n') {
                note.push('\n');
            }
            (Some(note), None)
        }
        Ok(Some(_)) => unread(format!("it is larger than {MAX_NOTE_BYTES} bytes")),
        Err(error) => unread(reason(&error)),
    }
}

/// The marker file's content.
#[derive(Debug, Serialize)]
struct Marker {
    kind: &'static str,
    version: u32,
}

/// The status-line hint, when the anneal home has no context usage and the hint was never
/// shown there; and a note when showing it could not be recorded.
///
/// Without an anneal home there is nowhere to remember the hint (and nowhere the status line
/// could save usage), so nothing is shown.
pub(crate) fn statusline_hint(
    lookup: impl Fn(&str) -> Option<OsString>,
) -> (Option<String>, Option<String>) {
    let Ok(home) = paths::anneal_home_from(lookup) else {
        return (None, None);
    };
    let usage = context::in_home(&home, context::USAGE_FILE);
    let marker = context::in_home(&home, context::HINT_FILE);
    if !is_missing(&usage) || !is_missing(&marker) {
        return (None, None);
    }
    let shown = Marker {
        kind: context::HINT_KIND,
        version: 1,
    };
    let note = baseline::write_snapshot_json(&marker, &shown)
        .err()
        .map(|error| {
            format!(
                "anneal could not record that the status-line hint was shown, so it may repeat: {}",
                reason(&error)
            )
        });
    (Some(format!("{STATUSLINE_HINT}\n")), note)
}

/// True only when nothing is at `path`. Anything else, including a path whose metadata cannot
/// be read, counts as present, so the hint errs toward silence.
fn is_missing(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}
