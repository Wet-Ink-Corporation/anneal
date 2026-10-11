//! The pending-decision state: one file per session, so sessions never share a write.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use serde_json::json;

use super::{OutcomeError, PENDING_DIR, Session, remember, settle};
use crate::decide::id::DecisionId;
use crate::decide::in_process::Prepared;
use crate::hook::clef_fake::{Fixed, TestResult, World};
use crate::hook::route::playbook::Playbook;

/// How many sessions run side by side in the concurrency test.
const SESSIONS: usize = 24;

fn prepared(world: &World) -> TestResult<Prepared> {
    Ok(Prepared::load(|name: &str| world.lookup(name))?)
}

/// A transcript for session `n` holding one line, and its path.
fn transcript(world: &World, n: usize) -> TestResult<PathBuf> {
    let path = world.home().join(format!("session-{n}.jsonl"));
    fs::write(&path, "{\"type\":\"user\"}\n")?;
    Ok(path)
}

/// Appends a `Skill` call for `skill` to the transcript at `path`.
fn invoke(path: &Path, skill: &str) -> TestResult {
    let line = json!({"type": "assistant", "message": {"id": "m", "role": "assistant",
        "content": [{"type": "tool_use", "id": "t", "name": "Skill", "input": {"skill": skill}}]}});
    let mut text = fs::read_to_string(path)?;
    text.push_str(&line.to_string());
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

/// Runs `step` for every session at once, one named scoped thread each, all joined.
fn all_at_once(
    sessions: &[Session<'_>],
    step: impl Fn(Session<'_>) -> Result<(), OutcomeError> + Sync,
) -> TestResult {
    thread::scope(|scope| {
        let step = &step;
        let mut handles = Vec::with_capacity(sessions.len());
        for (n, session) in sessions.iter().enumerate() {
            let handle = thread::Builder::new()
                .name(format!("outcome-test-{n}"))
                .spawn_scoped(scope, move || step(*session))?;
            handles.push(handle);
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_panic| "a session thread panicked")??;
        }
        Ok(())
    })
}

#[test]
fn each_session_keeps_its_own_file_named_by_a_hash_of_its_id() -> TestResult {
    let world = World::configured()?;
    let clef = prepared(&world)?;
    let path = transcript(&world, 1)?;
    let session = Session {
        id: "session-one",
        transcript: &path,
    };

    remember(&clef, session, &DecisionId::generate(1), "build")?;

    let dir = clef.log_dir().join(PENDING_DIR);
    let names: Vec<String> = fs::read_dir(&dir)?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    let [name] = names.as_slice() else {
        return Err(format!("one file per session: {names:?}").into());
    };
    let stem = name.strip_suffix(".json").ok_or("a .json file")?;
    assert_eq!(stem.len(), 64, "{name}");
    assert!(stem.bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
    let text = fs::read_to_string(dir.join(name))?;
    assert!(
        !text.contains("session-one"),
        "no session id on disk: {text}"
    );
    Ok(())
}

#[test]
fn sessions_remembered_and_settled_at_once_are_each_labelled_exactly_once() -> TestResult {
    let world = World::configured()?;
    let clef = prepared(&world)?;
    let playbook = Playbook::built_in()?;
    let ids: Vec<String> = (0..SESSIONS).map(|n| format!("session-{n}")).collect();
    let paths = (0..SESSIONS)
        .map(|n| transcript(&world, n))
        .collect::<TestResult<Vec<_>>>()?;
    let sessions: Vec<Session<'_>> = ids
        .iter()
        .zip(&paths)
        .map(|(id, path)| Session {
            id,
            transcript: path,
        })
        .collect();

    all_at_once(&sessions, |session| {
        remember(&clef, session, &DecisionId::generate(1), "build")
    })?;
    for path in &paths {
        invoke(path, "anneal:build")?;
    }
    for _round in 0..2 {
        all_at_once(&sessions, |session| {
            settle(&clef, &playbook, session, &Fixed)
        })?;
    }

    let labels: Vec<_> = world
        .rows()?
        .into_iter()
        .filter(|row| row["kind"] == json!("label"))
        .collect();
    let mut labelled: Vec<&str> = labels.iter().filter_map(|row| row["id"].as_str()).collect();
    labelled.sort_unstable();
    labelled.dedup();
    assert_eq!(labels.len(), SESSIONS, "none lost, none twice: {labels:?}");
    assert_eq!(labelled.len(), SESSIONS, "each decision once");
    assert!(labels.iter().all(|row| row["outcome"] == json!("build")));
    Ok(())
}

#[test]
fn past_the_session_bound_the_oldest_files_are_dropped() -> TestResult {
    let world = World::configured()?;
    let clef = prepared(&world)?;
    let path = transcript(&world, 0)?;
    let ids: Vec<String> = (0..super::MAX_SESSIONS + 3)
        .map(|n| format!("session-{n}"))
        .collect();

    for id in &ids {
        let session = Session {
            id,
            transcript: &path,
        };
        remember(&clef, session, &DecisionId::generate(1), "build")?;
    }

    let kept = fs::read_dir(clef.log_dir().join(PENDING_DIR))?.count();
    assert_eq!(kept, super::MAX_SESSIONS);
    Ok(())
}
