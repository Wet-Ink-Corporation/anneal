use std::error::Error;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anneal_ledger::event::{Mode, ModeSet, TaskDone};
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};
use serde_json::Value;

use super::{CONTINUE_HERE, run};
use crate::hook::HookEnv;
use crate::mode::scope;
use crate::paths;
use crate::test_repo::ScratchRepo;

type TestResult = Result<(), Box<dyn Error>>;

const PLAN: &str = "\
# Plan

## Goal

Ship it.

## Tasks

### T1: Parse the input

Files: `src/a.rs`
Test: `cargo test a`
Blocked by: none

### T2: Write the output

Files: `src/b.rs`
Test: `cargo test b`
Blocked by: T1

### T3: Document it

Files: `README.md`
Test: `cargo test c`
Blocked by: T2
";

/// A scratch repository with its own ledger file and anneal home.
struct Repo {
    repo: ScratchRepo,
    state: tempfile::TempDir,
}

impl Repo {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            repo: ScratchRepo::new()?,
            state: tempfile::tempdir()?,
        })
    }

    fn root(&self) -> &Path {
        self.repo.path()
    }

    fn store_path(&self) -> PathBuf {
        self.state.path().join("ledger.sqlite3")
    }

    /// Appends `event` tagged with the repository's worktree.
    fn append(&self, event: LedgerEvent) -> TestResult {
        let root = paths::worktree_root_within(self.root(), paths::GIT_TIMEOUT)?;
        let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
        envelope.worktree = Some(scope::worktree_key(&root));
        Store::open(&self.store_path())?.append(&envelope)?;
        Ok(())
    }

    /// Build mode on `plans/p.md`, with `T1` done.
    fn building(&self) -> TestResult {
        self.repo.write("plans/p.md", PLAN)?;
        self.repo.write("tree.json", "{}")?;
        self.repo.commit("plan")?;
        self.append(LedgerEvent::ModeSet(ModeSet {
            mode: Mode::Build,
            clean: true,
            plan: Some("plans/p.md".to_owned()),
            tree: Some("tree.json".to_owned()),
        }))?;
        self.append(LedgerEvent::TaskDone(TaskDone {
            task: "T1".to_owned(),
        }))
    }

    /// Runs the hook in `cwd` with a manual `PreCompact` input; its output, if any.
    fn handoff_in(&self, cwd: &Path) -> Option<Value> {
        let input = serde_json::json!({
            "session_id": "s1",
            "cwd": cwd,
            "hook_event_name": "PreCompact",
            "trigger": "manual",
            "custom_instructions": "keep the parser notes",
        })
        .to_string();
        let store = self.store_path();
        let home = self.state.path().join("home");
        let lookup = move |var: &str| match var {
            "ANNEAL_STORE" => Some(OsString::from(&store)),
            "ANNEAL_HOME" => Some(OsString::from(&home)),
            _ => None,
        };
        let mut out = Vec::new();
        let env = HookEnv {
            stdin: input.as_bytes(),
            current_dir: || Ok(PathBuf::new()),
            lookup,
        };
        run(env, &mut out).expect("a Vec accepts every write");
        (!out.is_empty()).then(|| serde_json::from_slice(&out).expect("one JSON object"))
    }

    fn handoff(&self) -> Option<Value> {
        self.handoff_in(self.root())
    }

    fn written(&self) -> String {
        std::fs::read_to_string(self.root().join(CONTINUE_HERE)).expect(".continue-here exists")
    }
}

#[test]
fn it_writes_the_phase_the_task_the_open_gates_and_the_ledger() -> TestResult {
    let repo = Repo::new()?;
    repo.building()?;

    let output = repo.handoff();

    assert_eq!(output, None, "silent when everything was read");
    let text = repo.written();
    for line in [
        "# Continue here\n",
        "- Phase: build mode (plan `plans/p.md`, design tree `tree.json`)\n",
        "- Task: `T2` (2 of 3): Write the output; 1 done\n",
        "- Open gates: none\n",
        "- Compact instructions: keep the parser notes\n",
    ] {
        assert!(text.contains(line), "{line:?} in:\n{text}");
    }
    let ledger = text
        .lines()
        .find(|line| line.starts_with("- Ledger (newest last): "))
        .expect("a ledger line");
    assert!(
        ledger.ends_with("mode_set build; task_done task:T1"),
        "the latest events, oldest first: {ledger}"
    );
    Ok(())
}

#[test]
fn a_leftover_is_an_open_gate() -> TestResult {
    let repo = Repo::new()?;
    repo.building()?;
    repo.repo.write("src/a.rs", "// [DEBUG-x1] probe\n")?;

    repo.handoff();

    let text = repo.written();
    let gates = text
        .lines()
        .find(|line| line.starts_with("- Open gates: "))
        .expect("a gates line");
    assert!(
        gates.contains("[DEBUG-<id>] instrumentation is left at src/a.rs:1"),
        "the Stop hook's gate: {gates}"
    );
    Ok(())
}

#[test]
fn notes_written_into_the_file_survive_the_next_handoff() -> TestResult {
    let repo = Repo::new()?;
    repo.building()?;
    repo.handoff();
    let mut text = repo.written();
    text.push_str("- decided: the parser keeps comments\n");
    std::fs::write(repo.root().join(CONTINUE_HERE), &text)?;

    repo.handoff();

    let again = repo.written();
    let notes = again.split_once("\n## Notes\n").map(|(_, notes)| notes);
    assert!(
        notes.is_some_and(|notes| notes.contains("- decided: the parser keeps comments\n")),
        "the notes are kept: {again}"
    );
    assert_eq!(
        again.matches("## Notes").count(),
        1,
        "one notes section: {again}"
    );
    Ok(())
}

#[test]
fn without_a_mode_the_phase_says_so_and_there_is_no_task() -> TestResult {
    let repo = Repo::new()?;

    repo.handoff();

    let text = repo.written();
    assert!(
        text.contains("- Phase: no anneal mode\n"),
        "no mode: {text}"
    );
    assert!(
        text.contains("- Task: no plan (only build mode has one)\n"),
        "no plan: {text}"
    );
    Ok(())
}

#[test]
fn outside_a_repository_nothing_is_written() -> TestResult {
    let repo = Repo::new()?;
    let elsewhere = tempfile::tempdir()?;

    let output = repo.handoff_in(elsewhere.path());

    assert_eq!(output, None, "silent");
    assert!(
        !elsewhere.path().join(CONTINUE_HERE).exists(),
        "no file outside a repository"
    );
    Ok(())
}
