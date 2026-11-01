//! `anneal hook bootstrap`: the `SessionStart` hook.
//!
//! It reads Claude Code's `SessionStart` input on stdin and writes one `SessionStart` output object
//! on stdout whose `additionalContext` is the [`index`] (stable, every time; only the skills
//! the plugin at `CLAUDE_PLUGIN_ROOT` holds, when it has a `skills/` directory), then the
//! worktree's `.continue-here` note when there is one (the `handoff` hook writes it before a
//! compaction), then a one-time status-line setup hint while no status line saves context
//! usage, then the live state from the ledger when it changed for this session (volatile,
//! last). With `CLAUDE_ENV_FILE` and `CLAUDE_PLUGIN_ROOT` set it also appends the line that
//! puts the plugin's `libexec` on PATH for later Bash calls ([`env_file`]). Every failure
//! becomes a one-line note in that context: the hook fails open.

mod compaction;
mod env_file;
pub(crate) mod index;
mod state;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::decide::clock::SystemClock;
use crate::hook::{HookEnv, reason};
use crate::paths;
use state::Sessions;

/// The most stdin a bootstrap reads; Claude Code's `SessionStart` input is far smaller (H-13).
pub(crate) const MAX_INPUT_BYTES: u64 = 64 * 1024;

/// How long git may take to find the ledger (`anneal ledger` allows `paths::GIT_TIMEOUT`, 10 s).
/// With the ledger's short busy timeout this keeps the hook well under its 15 s `timeout`, so
/// the index always arrives; see `docs/hooks.md`.
const GIT_TIMEOUT: Duration = Duration::from_secs(2);

/// The fields of Claude Code's `SessionStart` input the bootstrap uses; the rest are ignored, so
/// fields a newer Claude Code adds do not break it.
#[derive(Debug, Default, Deserialize)]
struct Input {
    /// The session's id; without one, the live state is injected every time.
    #[serde(default)]
    session_id: Option<String>,
    /// The session's working directory, where the ledger is resolved.
    #[serde(default)]
    cwd: Option<PathBuf>,
}

/// Why the hook input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    /// stdin could not be read.
    #[error("reading stdin")]
    Read(#[source] io::Error),
    /// stdin held more than [`MAX_INPUT_BYTES`].
    #[error("the input is larger than {MAX_INPUT_BYTES} bytes")]
    TooLarge,
    /// stdin was not a JSON object of the expected shape.
    #[error("the input is not SessionStart JSON")]
    Json(#[source] serde_json::Error),
}

/// `{"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": "..."}}`.
#[derive(Debug, Serialize)]
struct Output<'a> {
    #[serde(rename = "hookSpecificOutput")]
    specific: SessionStartOutput<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionStartOutput<'a> {
    hook_event_name: &'static str,
    additional_context: &'a str,
}

/// Runs the bootstrap: reads the input from `env.stdin`, resolves the ledger from the input's
/// `cwd` (or `env.current_dir` when it names none) and `env.lookup`, and writes the
/// `SessionStart` output to `out`. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`; every other failure is a note in the output.
pub(crate) fn run<R, D, L, W>(env: HookEnv<R, D, L>, mut out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    let context = context(read_input(env.stdin), env.current_dir, env.lookup);
    let output = Output {
        specific: SessionStartOutput {
            hook_event_name: "SessionStart",
            additional_context: &context,
        },
    };
    serde_json::to_writer(&mut out, &output)?;
    writeln!(out)
}

/// Reads and parses at most [`MAX_INPUT_BYTES`] of `stdin`.
fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let mut bytes = Vec::new();
    stdin
        .take(MAX_INPUT_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(InputError::Read)?;
    if !u64::try_from(bytes.len()).is_ok_and(|len| len <= MAX_INPUT_BYTES) {
        return Err(InputError::TooLarge);
    }
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// The index, then the live state when it changed, then any notes about what failed.
fn context(
    input: Result<Input, InputError>,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> String {
    let (input, input_note) = match input {
        Ok(input) => (input, None),
        Err(error) => (
            Input::default(),
            Some(format!(
                "anneal could not read the hook input: {}",
                reason(&error)
            )),
        ),
    };
    let cwd = match input.cwd {
        Some(cwd) => Ok(cwd),
        None => current_dir(),
    };
    let (handoff, handoff_note) = match &cwd {
        Ok(cwd) => compaction::continue_here(cwd, GIT_TIMEOUT, &lookup),
        // The live state's note below already says the working directory cannot be read.
        Err(_) => (None, None),
    };
    let (hint, hint_note) = compaction::statusline_hint(&lookup, &SystemClock::new());
    let path_note = env_file::export_libexec(&lookup);
    let (mut text, index_note) = plugin_index(&lookup);
    let (state, state_note) = match &cwd {
        Ok(cwd) => fresh_state(input.session_id.as_deref(), cwd, lookup),
        Err(error) => (
            None,
            Some(format!(
                "anneal skipped the live state: reading the working directory: {error}"
            )),
        ),
    };
    for section in [handoff, hint, state].into_iter().flatten() {
        text.push('\n');
        text.push_str(&section);
    }
    let notes = [
        input_note,
        index_note,
        path_note,
        handoff_note,
        hint_note,
        state_note,
    ];
    for note in notes.into_iter().flatten() {
        text.push('\n');
        text.push_str(&note);
        text.push('\n');
    }
    text
}

/// The index for the plugin at `$CLAUDE_PLUGIN_ROOT`, listing only the skills whose
/// `skills/<name>/SKILL.md` is a file there; and a note when a lookup failed. The hook fails
/// open to the full index: when the variable is unset or empty, when no skill is found, and,
/// with the note, when looking one up fails other than as not found.
fn plugin_index(lookup: impl Fn(&str) -> Option<OsString>) -> (String, Option<String>) {
    let Some(root) = lookup(env_file::PLUGIN_ROOT_VAR).filter(|root| !root.is_empty()) else {
        return (index::index(), None);
    };
    let skills = PathBuf::from(root).join("skills");
    let mut present = Vec::new();
    for name in index::skill_names() {
        let file = skills.join(name).join("SKILL.md");
        match std::fs::metadata(&file) {
            Ok(metadata) if metadata.is_file() => present.push(name),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                let note = format!(
                    "anneal listed every skill: looking up {}: {error}",
                    file.display()
                );
                return (index::index(), Some(note));
            }
        }
    }
    if present.is_empty() {
        return (index::index(), None);
    }
    (index::index_with(|name| present.contains(&name)), None)
}

/// The live state section for the ledger seen from `cwd`, unless `session` was already given
/// exactly this state; and a note when what the session was given could not be recorded.
fn fresh_state(
    session: Option<&str>,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> (Option<String>, Option<String>) {
    let store = match paths::ledger_store_within(None, lookup, cwd, GIT_TIMEOUT) {
        Ok(location) => location.path,
        Err(error) => return (Some(state::unavailable(&reason(&error))), None),
    };
    let state = match state::live_state(&store) {
        Ok(Some(state)) => state,
        Ok(None) => return (None, None),
        Err(error) => state::unavailable(&reason(&error)),
    };
    let Some(session) = session else {
        return (Some(state), None);
    };
    let file = state::sessions_path(&store);
    let recorded = Sessions::load(&file).and_then(|mut sessions| {
        if sessions.is_current(session, &state) {
            return Ok(false);
        }
        sessions.record(session, &state);
        sessions.save(&file).map(|()| true)
    });
    match recorded {
        Ok(true) => (Some(state), None),
        Ok(false) => (None, None),
        Err(error) => (
            Some(state),
            Some(format!(
                "anneal could not record what this session was shown, so the live state may \
                 repeat: {}",
                reason(&error)
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::hook::HookEnv;
    use anneal_ledger::{EventEnvelope, Store};
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};

    /// The injected context for `stdin`, with the ledger at `store` (if any).
    fn context_for(
        stdin: &[u8],
        current_dir: impl FnOnce() -> io::Result<PathBuf>,
        store: Option<&Path>,
    ) -> String {
        let lookup = |var: &str| {
            (var == "ANNEAL_STORE")
                .then_some(store)
                .flatten()
                .map(OsString::from)
        };
        let mut out = Vec::new();
        let env = HookEnv {
            stdin,
            current_dir,
            lookup,
        };
        run(env, &mut out).expect("a Vec accepts every write");
        let json: serde_json::Value = serde_json::from_slice(&out).expect("one JSON object");
        json.pointer("/hookSpecificOutput/additionalContext")
            .and_then(serde_json::Value::as_str)
            .expect("additionalContext")
            .to_owned()
    }

    #[test]
    fn an_unreadable_working_directory_skips_the_live_state_with_a_note() {
        let context = context_for(b"{}", || Err(io::Error::other("gone")), None);

        assert!(context.starts_with("# anneal\n"), "the index first");
        assert!(
            context.ends_with(
                "\nanneal skipped the live state: reading the working directory: gone\n"
            ),
            "the note last: {context}"
        );
    }

    #[test]
    fn a_session_start_puts_the_plugin_s_libexec_on_path_through_the_env_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let env_file = dir.path().join("env.sh");
        let lookup = |var: &str| match var {
            "CLAUDE_ENV_FILE" => Some(OsString::from(env_file.as_os_str())),
            "CLAUDE_PLUGIN_ROOT" => Some(OsString::from("/plugins/anneal")),
            _ => None,
        };
        let env = HookEnv {
            stdin: &b"{}"[..],
            current_dir: || Err(io::Error::other("gone")),
            lookup,
        };

        run(env, &mut Vec::new()).expect("a Vec accepts every write");

        assert_eq!(
            std::fs::read_to_string(&env_file).expect("the env file was written"),
            "export PATH='/plugins/anneal/libexec':\"$PATH\"\n"
        );
    }

    /// The injected context with `CLAUDE_PLUGIN_ROOT` set to `root`.
    fn context_with_plugin_root(root: &Path) -> String {
        let lookup = |var: &str| (var == "CLAUDE_PLUGIN_ROOT").then(|| OsString::from(root));
        let mut out = Vec::new();
        let env = HookEnv {
            stdin: &b"{}"[..],
            current_dir: || Err(io::Error::other("gone")),
            lookup,
        };
        run(env, &mut out).expect("a Vec accepts every write");
        let json: serde_json::Value = serde_json::from_slice(&out).expect("one JSON object");
        json.pointer("/hookSpecificOutput/additionalContext")
            .and_then(serde_json::Value::as_str)
            .expect("additionalContext")
            .to_owned()
    }

    #[test]
    fn the_index_lists_only_the_skills_the_plugin_root_holds() {
        let plugin = tempfile::tempdir().expect("a temporary directory");
        for skill in ["spec", "verify", "handoff"] {
            let dir = plugin.path().join("skills").join(skill);
            std::fs::create_dir_all(&dir).expect("a skill directory");
            std::fs::write(dir.join("SKILL.md"), skill).expect("a SKILL.md");
        }
        std::fs::create_dir_all(plugin.path().join("skills/build")).expect("no SKILL.md");

        let context = context_with_plugin_root(plugin.path());

        let listed: Vec<&str> = context
            .lines()
            .filter_map(|line| line.strip_prefix("- `"))
            .filter_map(|rest| rest.split_once('`').map(|(name, _)| name))
            .collect();
        assert_eq!(listed, ["spec", "verify", "/anneal:handoff"], "{context}");
    }

    #[test]
    fn a_plugin_root_with_no_skills_directory_gets_the_whole_index() {
        let plugin = tempfile::tempdir().expect("a temporary directory");

        let context = context_with_plugin_root(plugin.path());

        assert!(
            context.starts_with(&super::index::index()),
            "fails open to every skill: {context}"
        );
    }

    #[test]
    fn a_plugin_root_whose_skills_directory_holds_no_skill_gets_the_whole_index() {
        let plugin = tempfile::tempdir().expect("a temporary directory");
        std::fs::create_dir_all(plugin.path().join("skills/build")).expect("no SKILL.md");

        let context = context_with_plugin_root(plugin.path());

        assert!(
            context.starts_with(&super::index::index()),
            "no skill found is no reason to list none: {context}"
        );
    }

    /// A skill path that is neither there nor absent (here `skills/grill` is a file, so its
    /// `SKILL.md` cannot be looked up: `NotADirectory`) fails open to every skill, with a note.
    #[test]
    #[cfg(unix)]
    fn a_skill_that_cannot_be_looked_up_gets_the_whole_index_and_a_note() {
        let plugin = tempfile::tempdir().expect("a temporary directory");
        std::fs::create_dir_all(plugin.path().join("skills/spec")).expect("a skill directory");
        std::fs::write(plugin.path().join("skills/spec/SKILL.md"), "spec").expect("a SKILL.md");
        std::fs::write(plugin.path().join("skills/grill"), "not a directory").expect("a file");

        let context = context_with_plugin_root(plugin.path());

        assert_eq!(
            (
                context.starts_with(&super::index::index()),
                context.contains("\nanneal listed every skill: looking up "),
            ),
            (true, true),
            "{context}"
        );
    }

    #[test]
    fn without_a_session_id_the_live_state_is_injected_every_time() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = dir.path().join("ledger.sqlite3");
        let event = EventEnvelope::parse(
            br#"{"type":"task_done","task":"T1","actor":"cli","ts":"2026-10-11T09:30:00Z"}"#,
        )
        .expect("a valid event");
        Store::open(&store)
            .and_then(|ledger| ledger.append(&event))
            .expect("appended");
        let input = serde_json::json!({ "cwd": dir.path() }).to_string();

        let first = context_for(input.as_bytes(), || Ok(PathBuf::new()), Some(&store));
        let second = context_for(input.as_bytes(), || Ok(PathBuf::new()), Some(&store));

        let tail = "- #1 2026-10-11T09:30:00Z task_done task:T1\n";
        assert!(first.ends_with(tail), "first: {first}");
        assert!(
            second.ends_with(tail),
            "with no session id nothing records what was shown: {second}"
        );
    }
}
