//! Release 4's exit check, in process: one session asks Clef D1 (the guard), D3 (route), D6
//! (ruled-before) and D7 (`grill classify`) through their own entry points against one fake
//! Clef and one anneal home, and `anneal calibrate` reads all four points' rows back. The
//! binary's own run, where every call is unreachable, is `tests/it/release_check.rs`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::Parser as _;
use serde_json::{Value, json};

use crate::calibrate::{self, CalibrateEnv};
use crate::cli::{Cli, Command};
use crate::decide::DecideEnv;
use crate::decide::id::{DecisionId, OutcomeLabel};
use crate::decide::in_process::Prepared;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::grill;
use crate::hook::clef_fake::{Fixed, TestResult, World, reply};
use crate::hook::guard::{self, Ports};
use crate::hook::{HookEnv, route, ruled_before};

/// Answers every question its request asks: noul `yes` at 0.3, a choice's first option at 0.8
/// (the rest share 0.2), a score of 0.5.
struct Clef;

impl Transport for Clef {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        let unreadable = |what: &str, why: &str| TransportError::Unreachable {
            reason: format!("the fake cannot read the request's {what}: {why}"),
        };
        let body: Value = serde_json::from_slice(&request.body)
            .map_err(|error| unreadable("body", &error.to_string()))?;
        let system = body
            .pointer("/messages/0/content")
            .and_then(Value::as_str)
            .ok_or_else(|| unreadable("system message", "missing"))?;
        let asked: Value = serde_json::from_str(system)
            .map_err(|error| unreadable("question", &error.to_string()))?;
        let options: Vec<&str> = asked["options"]
            .as_array()
            .map(|options| options.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let answer = match (asked["question"].as_str(), options.split_first()) {
            (Some("score"), _) => json!({"score": 0.5}),
            (Some("choice"), Some((first, rest))) => {
                let share = 0.2 / f64::from(u32::try_from(rest.len()).unwrap_or(1).max(1));
                let mut probabilities = serde_json::Map::new();
                probabilities.insert((*first).to_owned(), json!(0.8));
                for option in rest {
                    probabilities.insert((*option).to_owned(), json!(share));
                }
                json!({ "probabilities": probabilities })
            }
            _ => json!({"probabilities": {"yes": 0.3, "no": 0.7}}),
        };
        reply(Ok(answer))
    }
}

/// A project with one out-of-scope ruling (for D6) and a design tree with one open question
/// (for D7).
fn project() -> TestResult<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join(".out-of-scope");
    std::fs::create_dir_all(&store)?;
    std::fs::write(
        store.join("licence.md"),
        "---\nid: licence\nruled_at: 2026-10-11\nsummary: MIT OR Apache-2.0.\n---\n",
    )?;
    let tree = json!({
        "version": 1,
        "goal": "export orders as CSV",
        "turns": [],
        "nodes": [{
            "id": "library", "question": "Which CSV library does the repository use?",
            "depends_on": [], "status": "open", "answer": "the one in Cargo.toml",
            "is_decision": 0.8, "confidence": 0.5
        }],
    });
    std::fs::write(dir.path().join("tree.json"), tree.to_string())?;
    Ok(dir)
}

/// The guard on a Bash call the denylist leaves to Clef.
fn ask_d1(world: &World) -> TestResult {
    let stdin = json!({
        "session_id": "s1", "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": "ls -la"},
    })
    .to_string();
    let env = HookEnv {
        stdin: stdin.as_bytes(),
        current_dir: || Ok(PathBuf::from("/")),
        lookup: |name: &str| world.lookup(name),
    };
    let ports = Ports {
        transport: &Clef,
        clock: &Fixed,
    };
    Ok(guard::run_with(env, &ports, &mut Vec::new())?)
}

/// Route, then ruled-before, on one prompt in `cwd`.
fn ask_d3_and_d6(world: &World, cwd: &Path) -> TestResult {
    let stdin = world.input("let us switch the licence to GPL", cwd);
    let env = || HookEnv {
        stdin: stdin.as_slice(),
        current_dir: || Ok(cwd.to_path_buf()),
        lookup: |name: &str| world.lookup(name),
    };
    route::run(env(), &Clef, &Fixed, Vec::new())?;
    ruled_before::run(env(), &Clef, &Fixed, Vec::new())?;
    Ok(())
}

/// `anneal grill classify` on the tree's open question; returns the decision's id.
fn ask_d7(world: &World, cwd: &Path) -> TestResult<String> {
    let words = [
        "anneal",
        "grill",
        "classify",
        "--tree",
        "tree.json",
        "--node",
    ];
    let parsed = Cli::try_parse_from(words.into_iter().chain(["library"]))?;
    let Some(Command::Grill(args)) = parsed.command else {
        return Err("not grill".into());
    };
    let env = DecideEnv {
        cwd,
        lookup: |name: &str| world.lookup(name),
        transport: &Clef,
        clock: &Fixed,
    };
    let mut out = Vec::new();
    grill::run(&args, &env, &mut out)?;
    let envelope: Value = serde_json::from_slice(&out)?;
    Ok(envelope["data"]["id"]
        .as_str()
        .ok_or("classify reports the decision's id")?
        .to_owned())
}

/// `anneal calibrate`'s envelope for `world`'s anneal home.
fn calibrate(world: &World) -> TestResult<Value> {
    let parsed = Cli::try_parse_from(["anneal", "calibrate"])?;
    let Some(Command::Calibrate(args)) = parsed.command else {
        return Err("not calibrate".into());
    };
    let env = CalibrateEnv {
        cwd: world.home(),
        lookup: |name: &str| -> Option<OsString> { world.lookup(name) },
    };
    let mut out = Vec::new();
    calibrate::run(&args, &env, &mut out, &mut Vec::new())?;
    Ok(serde_json::from_slice(&out)?)
}

#[test]
fn one_session_logs_d1_d3_d6_and_d7_and_calibrate_reads_all_four() -> TestResult {
    let world = World::configured()?;
    let project = project()?;

    ask_d1(&world)?;
    ask_d3_and_d6(&world, project.path())?;
    let d7 = ask_d7(&world, project.path())?;
    // The human: "you could have looked that up". D7 predicted no decision, rightly.
    let clef = Prepared::load(|name: &str| world.lookup(name))?;
    clef.label(
        &d7.parse::<DecisionId>()?,
        &"no".parse::<OutcomeLabel>()?,
        &Fixed,
    )?;
    let envelope = calibrate(&world)?;

    let mut logged: Vec<String> = world
        .rows()?
        .iter()
        .filter(|row| row["kind"] == json!("decision"))
        .map(|row| format!("{} {}", row["point"], row["question"]))
        .collect();
    // D3's two questions run concurrently, so their rows may land in either order.
    logged.sort();
    assert_eq!(
        logged,
        [
            r#""D1" "noul""#,
            r#""D3" "choice""#,
            r#""D3" "score""#,
            r#""D6" "choice""#,
            r#""D7" "noul""#,
        ],
        "one decision row per question asked"
    );
    let read: Vec<Value> = envelope["data"]["points"]
        .as_array()
        .ok_or("calibrate lists the points")?
        .iter()
        .flat_map(|point| {
            let questions = point["questions"].as_array().cloned().unwrap_or_default();
            questions.into_iter().map(move |question| {
                json!([
                    point["point"],
                    question["question"],
                    question["n"],
                    question["labelled"]
                ])
            })
        })
        .filter(|row| row[2] != json!(0))
        .collect();
    assert_eq!(
        read,
        [
            json!(["D1", "noul", 1, 0]),
            json!(["D3", "choice", 1, 0]),
            json!(["D3", "score", 1, 0]),
            json!(["D6", "choice", 1, 0]),
            json!(["D7", "noul", 1, 1]),
        ],
        "calibrate reads every point's answered decision, and D7's label"
    );
    Ok(())
}
