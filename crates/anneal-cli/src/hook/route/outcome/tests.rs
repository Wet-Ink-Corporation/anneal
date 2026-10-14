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

/// The files in `dir` with the extension `ext`.
fn with_extension(dir: &Path, ext: &str) -> TestResult<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == ext) {
            found.push(path);
        }
    }
    Ok(found)
}

/// Remembers a decision for each of `count` sessions named `<prefix>-<n>`.
fn remember_many(clef: &Prepared, transcript: &Path, prefix: &str, count: usize) -> TestResult {
    for n in 0..count {
        let id = format!("{prefix}-{n}");
        let session = Session {
            id: &id,
            transcript,
        };
        remember(clef, session, &DecisionId::generate(1), "build")?;
    }
    Ok(())
}

#[test]
fn a_claim_in_flight_survives_a_full_store() -> TestResult {
    let world = World::configured()?;
    let clef = prepared(&world)?;
    let path = transcript(&world, 0)?;
    remember_many(&clef, &path, "claimed", 1)?;
    let dir = clef.log_dir().join(PENDING_DIR);
    let [pending] = <[PathBuf; 1]>::try_from(with_extension(&dir, "json")?)
        .map_err(|files| format!("one pending file: {files:?}"))?;
    // Another hook has renamed it to claim it, and is still reading it.
    let claimed = pending.with_extension("4242.claimed");
    fs::rename(&pending, &claimed)?;

    remember_many(&clef, &path, "other", super::MAX_SESSIONS + 1)?;

    assert!(
        claimed.is_file(),
        "the oldest file, but claimed: not dropped"
    );
    assert_eq!(with_extension(&dir, "json")?.len(), super::MAX_SESSIONS);
    Ok(())
}

#[test]
fn a_claim_whose_file_vanished_is_nothing_to_label() -> TestResult {
    let world = World::configured()?;
    let gone = world.home().join("gone.4242.claimed");

    let taken = super::take(&gone)?;

    assert!(taken.is_none(), "{taken:?}");
    Ok(())
}

#[test]
fn leftover_claims_past_the_bound_are_dropped() -> TestResult {
    let world = World::configured()?;
    let clef = prepared(&world)?;
    let path = transcript(&world, 0)?;
    let dir = clef.log_dir().join(PENDING_DIR);
    fs::create_dir_all(&dir)?;
    // Claims left by hooks killed between claiming and removing.
    for n in 0..super::MAX_SESSIONS + 3 {
        fs::write(dir.join(format!("{n:064x}.{n}.claimed")), "{}")?;
    }

    remember_many(&clef, &path, "session", 1)?;

    assert_eq!(with_extension(&dir, "claimed")?.len(), super::MAX_SESSIONS);
    assert_eq!(with_extension(&dir, "json")?.len(), 1);
    Ok(())
}
