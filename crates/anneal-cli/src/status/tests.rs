//! `anneal status` in process, against a scratch repository, a ledger at `$ANNEAL_STORE` and an
//! anneal home at `$ANNEAL_HOME`, each in its own temporary directory. The JSON tests compare
//! the whole document (a snapshot); only `HEAD`'s commit id is read from the repository.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anneal_ledger::event::{
    GateBlocked, Mode, ModeSet, PlanApproved, StopBlocked, TaskDone, Verdict, VerdictSet,
    VerdictSource,
};
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};
use clap::Parser;
use serde_json::{Value, json};
use tempfile::TempDir;

mod edges;

use super::text::MAX_LINES;
use crate::cli::{Cli, Command};
use crate::content_hash::sha256;
use crate::context::sessions::SessionFile;
use crate::context::usage::WindowSource;
use crate::context::{self, Snapshot};
use crate::decide::log::DecisionLog;
use crate::decide::point::Point;
use crate::mode::scope;
use crate::paths::{self, GIT_TIMEOUT};
use crate::pr_diff;
use crate::test_repo::{ScratchRepo, Setup};

const PLAN: &str = "docs/plan.md";

const PLAN_TEXT: &str = "\
# Plan: export

- Spec: [export](../specs/export.md)

## Goal

Analysts export a report as CSV.

## Tasks

### T1: CSV writer

- Files: `src/csv.rs`
- Test: `cargo test csv`
- Blocked by: none

### T2: CLI flag

- Files: `src/cli.rs`
- Test: `cargo test cli`
- Blocked by: T1

## Review focus

- Quoting.
";

/// A scratch repository with the plan committed, an anneal home and a ledger path.
struct Fixture {
    repo: ScratchRepo,
    state: TempDir,
    vars: BTreeMap<&'static str, OsString>,
}

impl Fixture {
    fn new() -> Setup<Self> {
        let repo = ScratchRepo::new()?;
        repo.write(PLAN, PLAN_TEXT)?;
        repo.commit("plan")?;
        let state = tempfile::tempdir()?;
        let mut vars = BTreeMap::new();
        vars.insert("ANNEAL_STORE", state.path().join("ledger.sqlite3").into());
        vars.insert("ANNEAL_HOME", state.path().join("home").into());
        Ok(Self { repo, state, vars })
    }

    fn store_path(&self) -> PathBuf {
        self.state.path().join("ledger.sqlite3")
    }

    fn home(&self) -> PathBuf {
        self.state.path().join("home")
    }

    fn store(&self) -> Setup<Store> {
        Ok(Store::open(&self.store_path())?)
    }

    /// The worktree key events of this repository are tagged with.
    fn key(&self) -> Setup<String> {
        let root = paths::worktree_root_within(self.repo.path(), GIT_TIMEOUT)?;
        Ok(scope::worktree_key(&root))
    }

    /// Runs `anneal status <args>` in the repository: the exit code and stdout.
    fn status(&self, args: &[&str]) -> Setup<(u8, String)> {
        let cli = Cli::try_parse_from(["anneal", "status"].iter().chain(args))?;
        let Some(Command::Status(status)) = cli.command else {
            return Err("not the status command".into());
        };
        let mut out = Vec::new();
        let code = super::run(
            &status,
            self.repo.path(),
            |var| self.vars.get(var).cloned(),
            &mut out,
        )?;
        Ok((code.value(), String::from_utf8(out)?))
    }

    /// Runs `anneal status --json <args>`: the exit code and the envelope.
    fn json(&self, args: &[&str]) -> Setup<(u8, Value)> {
        let args: Vec<&str> = ["--json"].iter().chain(args).copied().collect();
        let (code, out) = self.status(&args)?;
        Ok((code, serde_json::from_str(&out)?))
    }
}

/// Appends `event` at `ts`, tagged with `worktree` when given; returns its position.
fn append(store: &Store, event: LedgerEvent, worktree: Option<&str>, ts: &str) -> Setup<u64> {
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::try_from(ts.to_owned())?);
    envelope.worktree = worktree.map(str::to_owned);
    Ok(store.append(&envelope)?)
}

fn build_mode(store: &Store, key: &str) -> Setup<u64> {
    let event = LedgerEvent::ModeSet(ModeSet {
        mode: Mode::Build,
        clean: true,
        plan: Some(PLAN.to_owned()),
        tree: Some("docs/tree.json".to_owned()),
    });
    append(store, event, Some(key), "2026-10-19T10:00:01Z")
}

fn approve(store: &Store, hash: &str) -> Setup<u64> {
    let event = LedgerEvent::PlanApproved(PlanApproved {
        plan: PLAN.to_owned(),
        hash: hash.to_owned(),
    });
    append(store, event, None, "2026-10-19T10:00:02Z")
}

/// The anneal home with three answered D1 decisions and one context reading at 60%.
fn home_state(home: &Path) -> Setup<()> {
    let log = DecisionLog::in_home(home);
    for _ in 0..3 {
        log.count_answered(Point::D1)?;
    }
    let path = context::in_home(home, context::USAGE_FILE);
    let mut file = SessionFile::<Snapshot>::load(&path, context::USAGE_KIND)?;
    file.set(
        "s1",
        Snapshot {
            used_tokens: Some(120_000.0),
            window: 200_000,
            window_source: WindowSource::Model,
        },
    );
    Ok(file.save(&path)?)
}

/// A worktree in build mode with an approved plan, one of two tasks done, a verdict for
/// `HEAD`, Clef configured and a context reading. Returns `HEAD`'s commit id.
fn mid_build(fixture: &mut Fixture) -> Setup<String> {
    let store = fixture.store()?;
    let key = fixture.key()?;
    let head = fixture.repo.head()?;
    build_mode(&store, &key)?;
    approve(&store, &sha256(PLAN_TEXT.as_bytes()))?;
    let done = LedgerEvent::TaskDone(TaskDone {
        task: "T1".to_owned(),
    });
    append(&store, done, Some(&key), "2026-10-19T10:00:03Z")?;
    let verdict = LedgerEvent::VerdictSet(VerdictSet {
        pr: 7,
        verdict: Verdict::TestsVerified,
        source: Some(VerdictSource::Local),
        sha: Some(head.clone()),
        patch_id: None,
        base: Some("main".to_owned()),
    });
    append(&store, verdict, None, "2026-10-19T10:00:04Z")?;
    home_state(&fixture.home())?;
    fixture
        .vars
        .insert("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct123".into());
    fixture
        .vars
        .insert("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "shadow,D1=enforce".into());
    Ok(head)
}

#[test]
fn an_empty_ledger_has_schema_1_no_gates_and_an_empty_tail() -> Setup<()> {
    let fixture = Fixture::new()?;
    fixture.store()?;

    let (code, envelope) = fixture.json(&[])?;

    assert_eq!(code, 0, "a ledger exists: {envelope}");
    assert_eq!(
        envelope["data"],
        json!({"schema": 1, "gates": [], "ledger_tail": []}),
        "no mode, plan, verdict, Clef or context reading: those sections are absent"
    );
    assert_eq!(envelope["findings"], json!([]), "nothing was unreadable");
    assert_eq!(envelope["ok"], json!(true), "ok agrees with exit 0");
    Ok(())
}

#[test]
fn without_a_ledger_it_exits_2_with_the_document_and_creates_none() -> Setup<()> {
    let fixture = Fixture::new()?;

    let (code, envelope) = fixture.json(&[])?;

    assert_eq!(code, 2, "no ledger: {envelope}");
    assert_eq!(
        envelope["data"],
        json!({"schema": 1}),
        "no section has data"
    );
    assert_eq!(
        envelope["findings"][0]["key"],
        json!("status/no-ledger"),
        "the finding names the missing ledger"
    );
    assert!(!fixture.store_path().exists(), "no ledger was created");
    Ok(())
}

#[test]
fn a_mid_build_ledger_folds_into_every_section() -> Setup<()> {
    let mut fixture = Fixture::new()?;
    let head = mid_build(&mut fixture)?;
    let hash = sha256(PLAN_TEXT.as_bytes());

    let (code, envelope) = fixture.json(&[])?;

    assert_eq!(code, 0, "{envelope}");
    let mut expected = mid_build_worktree(&head, &hash);
    let (Some(fields), Value::Object(rest)) = (expected.as_object_mut(), mid_build_rest()) else {
        return Err("both halves are objects".into());
    };
    fields.extend(rest);
    assert_eq!(envelope["data"], expected, "the whole document");
    assert_eq!(envelope["findings"], json!([]), "everything was readable");
    Ok(())
}

/// The mid-build document's worktree sections: phase, plan, tasks and verdict.
fn mid_build_worktree(head: &str, hash: &str) -> Value {
    json!({
            "schema": 1,
            "phase": {
                "mode": "build",
                "playbook": {"name": "build", "requires": ["test_run"]},
                "event": 1,
                "plan": PLAN,
                "tree": "docs/tree.json",
                "clean": true,
            },
            "plan": {
                "id": PLAN,
                "status": "approved",
                "hash": hash,
                "approved": {"hash": hash, "event": 2, "ts": "2026-10-19T10:00:02Z"},
            },
            "tasks": {"done": 1, "total": 2, "next": {"id": "T2", "title": "CLI flag"}},
            "verdict": {
                "pr": 7,
                "head": head,
                "base": "main",
                "outcome": "valid",
                "valid": true,
                "verdict": {
                    "event": 4,
                    "verdict": "tests_verified",
                    "source": "local",
                    "sha": head,
                    "fresh": "by_sha",
                },
                "stale": 0,
            },
    })
}

/// The mid-build document's other sections: gates, clef, context and the ledger's tail.
fn mid_build_rest() -> Value {
    let shadow = |point: &str| json!({"point": point, "answered": 0, "configured_mode": "shadow", "mode": "shadow"});
    json!({
            "gates": [],
            "clef": {
                "configured": true,
                "points": [
                    {"point": "D1", "answered": 3, "configured_mode": "enforce", "mode": "shadow"},
                    shadow("D2"),
                    shadow("D3"),
                    shadow("D4"),
                    shadow("D5"),
                    shadow("D6"),
                    shadow("D7"),
                ],
            },
            "context": {
                "used_tokens": 120_000.0,
                "window": 200_000,
                "window_source": "model",
                "percent": 60.0,
                "level": "warn",
            },
            "ledger_tail": [
                {"event": 1, "type": "mode_set", "summary": "mode_set build", "actor": "cli",
                 "ts": "2026-10-19T10:00:01Z"},
                {"event": 2, "type": "plan_approved", "summary": "plan_approved plan:docs/plan.md",
                 "actor": "cli", "ts": "2026-10-19T10:00:02Z"},
                {"event": 3, "type": "task_done", "summary": "task_done task:T1", "actor": "cli",
                 "ts": "2026-10-19T10:00:03Z"},
                {"event": 4, "type": "verdict_set", "summary": "verdict_set pr:7", "actor": "cli",
                 "ts": "2026-10-19T10:00:04Z"},
            ],
    })
}

#[test]
fn a_stale_plan_and_blocked_gates_are_shown_with_their_events() -> Setup<()> {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let key = fixture.key()?;
    build_mode(&store, &key)?;
    let approved = sha256(PLAN_TEXT.as_bytes());
    approve(&store, &approved)?;
    let edited = PLAN_TEXT.replace("CLI flag", "CLI option");
    fixture.repo.write(PLAN, &edited)?;
    let gate = LedgerEvent::GateBlocked(GateBlocked {
        point: "preconditions.plan".to_owned(),
        reason: "build mode: the plan changed since it was approved".to_owned(),
    });
    append(&store, gate, None, "2026-10-19T10:00:03Z")?;
    let stop = LedgerEvent::StopBlocked(StopBlocked {
        point: "completeness.claim".to_owned(),
        reason: "a success claim with no green test run".to_owned(),
    });
    append(&store, stop, None, "2026-10-19T10:00:04Z")?;

    let (code, envelope) = fixture.json(&["--fields", "plan,tasks,gates"])?;

    assert_eq!(code, 0, "{envelope}");
    assert_eq!(
        envelope["data"],
        json!({
            "schema": 1,
            "plan": {
                "id": PLAN,
                "status": "stale",
                "hash": sha256(edited.as_bytes()),
                "approved": {"hash": approved, "event": 2, "ts": "2026-10-19T10:00:02Z"},
            },
            "tasks": {"done": 0, "total": 2, "next": {"id": "T1", "title": "CSV writer"}},
            "gates": [
                {"id": "preconditions.plan", "kind": "gate_blocked",
                 "reason": "build mode: the plan changed since it was approved", "event": 3,
                 "ts": "2026-10-19T10:00:03Z"},
                {"id": "completeness.claim", "kind": "stop_blocked",
                 "reason": "a success claim with no green test run", "event": 4,
                 "ts": "2026-10-19T10:00:04Z"},
            ],
        }),
        "the whole document"
    );
    let (_, text) = fixture.status(&["--fields", "plan,gates"])?;
    assert_eq!(
        text,
        "plan: stale docs/plan.md\ngates: 2 recent blocks, newest completeness.claim (#4): a \
         success claim with no green test run\n",
        "the text form names the stale plan and the newest block"
    );
    Ok(())
}

/// A verdict set for the `feature` branch's first head, with its patch id from `main`.
fn feature_verdict(fixture: &Fixture) -> Setup<String> {
    let main = fixture.repo.head()?;
    let head = fixture.repo.feature()?;
    let patch_id = pr_diff::patch_id(fixture.repo.path(), &main, &head, GIT_TIMEOUT)?;
    let verdict = LedgerEvent::VerdictSet(VerdictSet {
        pr: 9,
        verdict: Verdict::Reviewed,
        source: Some(VerdictSource::ReviewPlugin),
        sha: Some(head.clone()),
        patch_id,
        base: Some("main".to_owned()),
    });
    append(&fixture.store()?, verdict, None, "2026-10-19T10:00:01Z")?;
    Ok(head)
}

#[test]
fn a_verdict_kept_by_a_rebase_is_fresh_by_patch_id() -> Setup<()> {
    let fixture = Fixture::new()?;
    let set_for = feature_verdict(&fixture)?;
    let head = fixture.repo.rebase_feature()?;

    let (_, envelope) = fixture.json(&["--fields", "verdict"])?;

    assert_eq!(
        envelope["data"]["verdict"],
        json!({
            "pr": 9,
            "head": head,
            "base": "main",
            "outcome": "valid",
            "valid": true,
            "verdict": {
                "event": 1,
                "verdict": "reviewed",
                "source": "review_plugin",
                "sha": set_for,
                "fresh": "by_patch_id",
            },
            "stale": 0,
        }),
        "the rebase kept the patch id, so the verdict still holds"
    );
    Ok(())
}

#[test]
fn a_verdict_for_other_code_is_stale_and_not_valid() -> Setup<()> {
    let fixture = Fixture::new()?;
    feature_verdict(&fixture)?;
    let head = fixture.repo.change_feature()?;

    let (_, envelope) = fixture.json(&["--fields", "verdict"])?;

    assert_eq!(
        envelope["data"]["verdict"],
        json!({
            "pr": 9,
            "head": head,
            "base": "main",
            "outcome": "stale",
            "valid": false,
            "stale": 1,
        }),
        "the code changed, so no verdict is fresh"
    );
    Ok(())
}

#[test]
fn fields_limit_the_document_to_the_sections_named() -> Setup<()> {
    let mut fixture = Fixture::new()?;
    mid_build(&mut fixture)?;

    let (_, envelope) = fixture.json(&["--fields", "phase,plan,gates"])?;

    let keys: Vec<&String> = envelope["data"]
        .as_object()
        .ok_or("data is an object")?
        .keys()
        .collect();
    assert_eq!(
        keys,
        ["gates", "phase", "plan", "schema"],
        "only those, and the schema"
    );
    Ok(())
}

#[test]
fn tail_lists_the_newest_events_oldest_first() -> Setup<()> {
    let mut fixture = Fixture::new()?;
    mid_build(&mut fixture)?;

    let (_, envelope) = fixture.json(&["--fields", "ledger_tail", "--tail", "2"])?;

    let events: Vec<&Value> = envelope["data"]["ledger_tail"]
        .as_array()
        .ok_or("ledger_tail is an array")?
        .iter()
        .map(|event| &event["event"])
        .collect();
    assert_eq!(
        events,
        [&json!(3), &json!(4)],
        "the two newest, oldest first"
    );
    Ok(())
}

#[test]
fn an_unreadable_source_is_a_finding_and_its_section_is_absent() -> Setup<()> {
    let fixture = Fixture::new()?;
    fixture.store()?;
    std::fs::create_dir_all(fixture.home())?;
    std::fs::write(
        context::in_home(&fixture.home(), context::USAGE_FILE),
        "not json",
    )?;

    let (code, envelope) = fixture.json(&["--fields", "context"])?;

    assert_eq!(code, 0, "an unreadable section does not fail the command");
    assert_eq!(envelope["data"], json!({"schema": 1}), "context is absent");
    assert_eq!(
        envelope["findings"][0]["key"],
        json!("status/unreadable:context"),
        "one finding names the section: {envelope}"
    );
    Ok(())
}

#[test]
fn the_text_form_is_one_line_per_section_within_12_lines() -> Setup<()> {
    let mut fixture = Fixture::new()?;
    mid_build(&mut fixture)?;

    let (code, text) = fixture.status(&[])?;

    assert_eq!(code, 0, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() <= MAX_LINES, "at most 12 lines: {text}");
    assert_eq!(
        lines,
        [
            "phase: build (plan docs/plan.md)",
            "plan: approved docs/plan.md",
            "tasks: 1 of 2 done, next T2",
            "gates: none blocked",
            "verdict: pr 7 valid (tests_verified)",
            "clef: D1 3 shadow, D2 0 shadow, D3 0 shadow, D4 0 shadow, D5 0 shadow, D6 0 shadow, \
             D7 0 shadow",
            "context: 120k/200k 60%",
            "ledger: #1 mode_set build; #2 plan_approved plan:docs/plan.md; #3 task_done \
             task:T1; #4 verdict_set pr:7",
        ],
        "one line per section, in the document's order"
    );
    Ok(())
}
