//! `anneal hook handoff`: the `PreCompact` hook. It fails open and never blocks compaction.
//!
//! Before Claude Code compacts, it writes [`CONTINUE_HERE`] at the root of the session's
//! worktree: the anneal mode (phase), the build plan's current task ("task N of M"), the open
//! gates (what the Stop hook would hold against stopping now: `[DEBUG-` leftovers and the
//! playbook's missing events), a one-line summary of the ledger's newest events, and the
//! instructions given to `/compact`. A `## Notes` section written into the previous file (what
//! a script cannot know) is kept, at most [`MAX_NOTES_BYTES`] of it. The bootstrap injects the
//! file at the next session start. Outside a git repository it writes nothing; it never
//! creates a ledger.
//!
//! Run by hand with `{}` on stdin, it writes the same file for a handoff without compaction
//! (the `/handoff` skill does this before `/clear`). What it cannot read is listed in the file,
//! and reported as one `ERROR` line in the `systemMessage` (which Claude Code discards for
//! `PreCompact`, but a manual run shows).

mod facts;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anneal_ledger::Timestamp;
use serde::Deserialize;

use crate::baseline;
use crate::error_chain::reason;
use crate::hook::stop_output::StopOutput;
use crate::hook::{HookEnv, StdinError, read_stdin};
use crate::paths::{self, PathError};
use facts::Facts;

/// The handoff note's name, at the worktree root (gitignored).
pub(crate) const CONTINUE_HERE: &str = ".continue-here";

/// The heading whose section, and everything after it, survives a rewrite.
const NOTES_HEADING: &str = "## Notes";

/// The most of the previous file's notes kept, in bytes (H-13).
pub(crate) const MAX_NOTES_BYTES: usize = 8 * 1024;

/// The most characters of the `/compact` instructions the note keeps.
const MAX_INSTRUCTION_CHARS: usize = 500;

/// The most stdin read; the `/compact` instructions ride in it (H-13).
const MAX_INPUT_BYTES: u64 = 1024 * 1024;

/// How long git may take to find the worktree.
const GIT_TIMEOUT: Duration = Duration::from_secs(3);

/// The fields of the `PreCompact` input the hook uses; the rest are ignored.
#[derive(Debug, Default, Deserialize)]
struct Input {
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default)]
    custom_instructions: Option<String>,
}

/// Why the input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error(transparent)]
    Stdin(#[from] StdinError),
    #[error("the input is not PreCompact JSON")]
    Json(#[source] serde_json::Error),
}

/// Runs the hook: reads the input from `env.stdin`, writes [`CONTINUE_HERE`] in the worktree of
/// its `cwd` (or `env.current_dir`), and writes any failure to `out` as one `ERROR` line.
///
/// # Errors
/// Only the I/O error from writing `out`.
pub(crate) fn run<R, D, L, W>(env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    let failed = |what: &str, error: &dyn std::error::Error| {
        StopOutput::new("PreCompact").error(&format!("handoff: {what}: {}", reason(error)))
    };
    let mut input = match read_stdin(env.stdin, MAX_INPUT_BYTES)
        .map_err(InputError::from)
        .and_then(|bytes| serde_json::from_slice::<Input>(&bytes).map_err(InputError::Json))
    {
        Ok(input) => input,
        Err(error) => return failed("the hook input cannot be read", &error).write(out),
    };
    let cwd = match input.cwd.take() {
        Some(cwd) => cwd,
        None => match (env.current_dir)() {
            Ok(cwd) => cwd,
            Err(error) => return failed("the working directory cannot be read", &error).write(out),
        },
    };
    let root = match paths::worktree_root_within(&cwd, GIT_TIMEOUT) {
        Ok(root) => root,
        Err(PathError::NotARepository { .. }) => return Ok(()),
        Err(error) => return failed("the worktree cannot be found", &error).write(out),
    };
    let problems = write_note(&root, &input, env.lookup);
    if problems.is_empty() {
        return Ok(());
    }
    StopOutput::new("PreCompact")
        .error(&format!("handoff: {}", problems.join("; ")))
        .write(out)
}

/// Writes the note at the worktree `root`; what could not be read or written.
fn write_note(root: &str, input: &Input, lookup: impl Fn(&str) -> Option<OsString>) -> Vec<String> {
    let path = Path::new(root).join(CONTINUE_HERE);
    let mut facts = facts::gather(root, lookup);
    let notes = match previous_notes(&path) {
        Ok(notes) => notes,
        Err(error) => {
            facts
                .problems
                .push(format!("the previous {CONTINUE_HERE}: {}", reason(&error)));
            None
        }
    };
    let written_at = match Timestamp::now() {
        Ok(now) => now.as_str().to_owned(),
        Err(error) => {
            facts
                .problems
                .push(format!("the clock: {}", reason(&error)));
            "an unknown time".to_owned()
        }
    };
    let text = render(&facts, input, &written_at, notes.as_deref());
    if let Err(error) = baseline::write_text_atomically(&path, &text) {
        facts.problems.push(reason(&error));
    }
    facts.problems
}

/// The `## Notes` section of the note at `path` and all after it, cut to [`MAX_NOTES_BYTES`];
/// `None` when there is no file or no such section.
fn previous_notes(path: &Path) -> Result<Option<String>, baseline::BaselineError> {
    let Some(bytes) = baseline::read_state_bytes(path)? else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&bytes);
    let start = text
        .match_indices(NOTES_HEADING)
        .map(|(at, _)| at)
        .find(|at| *at == 0 || text.get(..*at).is_some_and(|before| before.ends_with('\n')));
    Ok(start.and_then(|at| text.get(at..)).map(cut_notes))
}

/// `notes` cut to at most [`MAX_NOTES_BYTES`] at a character boundary, ending in a newline.
fn cut_notes(notes: &str) -> String {
    let mut used = 0_usize;
    let mut kept: String = notes
        .chars()
        .take_while(|c| {
            used = used.saturating_add(c.len_utf8());
            used <= MAX_NOTES_BYTES
        })
        .collect();
    if !kept.ends_with('\n') {
        kept.push('\n');
    }
    kept
}

/// The note's text.
fn render(facts: &Facts, input: &Input, written_at: &str, notes: Option<&str>) -> String {
    let when = match input.trigger.as_deref() {
        Some("manual") => "before a manual compaction",
        Some("auto") => "before an automatic compaction",
        _ => "for a handoff",
    };
    let gates = if facts.gates.is_empty() {
        "none".to_owned()
    } else {
        facts.gates.join("; ")
    };
    let mut items = vec![
        format!("- Phase: {}", facts.phase),
        format!("- Task: {}", facts.task),
        format!("- Open gates: {gates}"),
        format!("- Ledger (newest last): {}", facts.ledger),
    ];
    let instructions = input
        .custom_instructions
        .as_deref()
        .map(|given| given.replace(['\r', '\n'], " "))
        .filter(|given| !given.trim().is_empty());
    if let Some(given) = instructions {
        let given: String = given.chars().take(MAX_INSTRUCTION_CHARS).collect();
        items.push(format!("- Compact instructions: {given}"));
    }
    if !facts.problems.is_empty() {
        items.push(format!("- Not read: {}", facts.problems.join("; ")));
    }
    format!(
        "# Continue here\n\n\
         Written by anneal's handoff hook {when} at {written_at}. The next session start \
         injects it. Delete it once the work below is picked up.\n\n{}\n\n{}",
        items.join("\n"),
        notes.unwrap_or(
            "## Notes\n\nWhat a script cannot know: decisions in flight, what was tried, the \
             next step. This section is kept when the file is rewritten.\n"
        )
    )
}

#[cfg(test)]
mod tests;
