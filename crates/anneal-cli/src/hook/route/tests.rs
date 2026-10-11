//! `anneal hook route` with a fake Clef: injection only in enforce mode above the threshold,
//! silence otherwise, and the next prompt labelling the decision.

use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use super::run;
use crate::decide::clock::Clock;
use crate::decide::point::Point;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::hook::HookEnv;
use crate::hook::clef_fake::{
    Fixed, Stepping, TestResult, World, block_the_log, context, parsed, reply,
};

/// Answers the phase choice and the size score with fixed replies.
struct Endpoint {
    choice: Result<Value, TransportError>,
    score: Value,
}

impl Endpoint {
    fn answering(choice: Value, score: f64) -> Self {
        Self {
            choice: Ok(choice),
            score: json!({ "score": score }),
        }
    }
}

impl Transport for Endpoint {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        let body = String::from_utf8_lossy(&request.body);
        // Each call hands out its own copy of the fixed reply.
        if body.contains(r#"\"question\":\"score\""#) {
            reply(Ok(self.score.clone()))
        } else {
            reply(self.choice.clone())
        }
    }
}

/// Runs the hook on `prompt` in `world`, returning its stdout parsed (`Null` when silent).
fn route(world: &World, prompt: &str, transport: &(impl Transport + Sync)) -> TestResult<Value> {
    route_at(world, prompt, transport, &Fixed)
}

/// [`route`] with `clock`.
fn route_at(
    world: &World,
    prompt: &str,
    transport: &(impl Transport + Sync),
    clock: &(impl Clock + Sync),
) -> TestResult<Value> {
    let stdin = world.input(prompt, world.home());
    let env = HookEnv {
        stdin: stdin.as_slice(),
        current_dir: || Ok(PathBuf::new()),
        lookup: |name: &str| world.lookup(name),
    };
    let mut out = Vec::new();
    run(env, transport, clock, &mut out)?;
    parsed(&out)
}

fn build_choice(top: f64) -> Value {
    json!({"probabilities": {
        "build": top, "plan": 1.0 - top, "define": 0.0, "specify": 0.0,
        "diagnose": 0.0, "verify": 0.0, "ship": 0.0, "answer": 0.0
    }})
}

fn skill_call(skill: &str) -> Value {
    json!({"type": "assistant", "message": {"id": "m", "role": "assistant", "content": [
        {"type": "tool_use", "id": "t", "name": "Skill", "input": {"skill": skill}}
    ]}})
}

fn earlier() -> Value {
    json!({"type": "user", "message": {"content": "earlier"}})
}

fn of_kind<'r>(rows: &'r [Value], kind: &str) -> Vec<&'r Value> {
    rows.iter()
        .filter(|row| row["kind"] == json!(kind))
        .collect()
}

fn choices(rows: &[Value]) -> Vec<&Value> {
    rows.iter()
        .filter(|row| row["question"] == json!("choice"))
        .collect()
}

#[test]
fn with_clef_off_the_hook_writes_nothing_and_logs_nothing() -> TestResult {
    let world = World::new(&[])?;

    let output = route(
        &world,
        "build T3",
        &Endpoint::answering(build_choice(0.9), 0.5),
    )?;

    assert_eq!(output, Value::Null);
    assert_eq!(
        std::fs::read_dir(world.home())?.count(),
        0,
        "nothing created in the home"
    );
    Ok(())
}

#[test]
fn in_shadow_an_answer_over_the_threshold_is_logged_but_not_injected() -> TestResult {
    let world = World::configured()?;

    let output = route(
        &world,
        "build T3",
        &Endpoint::answering(build_choice(0.9), 0.5),
    )?;

    assert_eq!(output, Value::Null, "shadow: log only");
    let rows = world.rows()?;
    let mut questions: Vec<&str> = rows
        .iter()
        .filter_map(|row| row["question"].as_str())
        .collect();
    questions.sort_unstable();
    assert_eq!(questions, ["choice", "score"], "{rows:?}");
    let choice = choices(&rows);
    assert_eq!(choice[0]["action"], json!("hint"));
    assert_eq!(choice[0]["mode"], json!("shadow"));
    Ok(())
}

#[test]
fn in_enforce_an_answer_over_the_threshold_injects_one_line_naming_phase_depth_and_skill()
-> TestResult {
    let world = World::enforcing(Point::D3)?;

    let output = route(
        &world,
        "build T3",
        &Endpoint::answering(build_choice(0.9), 0.5),
    )?;

    assert_eq!(
        context(&output),
        Some(
            "anneal route: load the `build` skill before acting (phase build, size medium, \
             depth standard)."
        ),
        "{output}"
    );
    Ok(())
}

#[test]
fn in_enforce_an_answer_under_the_threshold_injects_nothing() -> TestResult {
    let world = World::enforcing(Point::D3)?;

    let output = route(
        &world,
        "build T3",
        &Endpoint::answering(build_choice(0.55), 0.5),
    )?;

    assert_eq!(output, Value::Null);
    Ok(())
}

#[test]
fn in_enforce_a_phase_without_an_entry_skill_injects_nothing() -> TestResult {
    let world = World::enforcing(Point::D3)?;
    let answer = json!({"probabilities": {
        "answer": 0.9, "build": 0.1, "plan": 0.0, "define": 0.0, "specify": 0.0,
        "diagnose": 0.0, "verify": 0.0, "ship": 0.0
    }});

    let output = route(
        &world,
        "what is a ledger?",
        &Endpoint::answering(answer, 0.0),
    )?;

    assert_eq!(output, Value::Null);
    Ok(())
}

#[test]
fn an_unreachable_clef_injects_nothing_and_logs_the_miss() -> TestResult {
    let world = World::enforcing(Point::D3)?;
    let endpoint = Endpoint {
        choice: Err(TransportError::TimedOut {
            after: Duration::from_secs(2),
        }),
        score: json!({"score": 0.5}),
    };

    let output = route(&world, "build T3", &endpoint)?;

    assert_eq!(output, Value::Null, "fails open, silently");
    let rows = world.rows()?;
    assert_eq!(choices(&rows)[0]["unreachable"], json!(true));
    Ok(())
}

#[test]
fn a_malformed_configuration_is_one_error_line_and_no_context() -> TestResult {
    let mut vars = crate::hook::clef_fake::CONFIGURED.to_vec();
    vars.push(("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "loud"));
    let world = World::new(&vars)?;

    let output = route(
        &world,
        "build T3",
        &Endpoint::answering(build_choice(0.9), 0.5),
    )?;

    assert_eq!(context(&output), None);
    let message = output["systemMessage"].as_str().unwrap_or_default();
    assert!(message.starts_with("ERROR route: "), "{output}");
    assert!(world.rows()?.is_empty(), "nothing asked, nothing logged");
    Ok(())
}

#[test]
fn an_empty_prompt_asks_nothing() -> TestResult {
    let world = World::configured()?;

    let output = route(
        &world,
        "  \n ",
        &Endpoint::answering(build_choice(0.9), 0.5),
    )?;

    assert_eq!(output, Value::Null);
    assert!(world.rows()?.is_empty(), "nothing asked");
    Ok(())
}

#[test]
fn the_next_prompt_labels_the_decision_with_its_phase_when_the_skill_was_invoked() -> TestResult {
    let world = World::configured()?;
    let endpoint = Endpoint::answering(build_choice(0.9), 0.5);
    world.append_transcript(&[earlier()])?;
    route(&world, "build T3", &endpoint)?;
    world.append_transcript(&[skill_call("anneal:build")])?;

    route(&world, "next", &endpoint)?;

    let rows = world.rows()?;
    let first = choices(&rows)[0];
    let labels = of_kind(&rows, "label");
    assert_eq!(labels.len(), 1, "{rows:?}");
    assert_eq!(labels[0]["id"], first["id"]);
    assert_eq!(
        labels[0]["outcome"],
        json!("build"),
        "the hinted skill was invoked"
    );
    Ok(())
}

#[test]
fn another_skill_labels_its_phase_and_no_skill_labels_the_no_skill_phase() -> TestResult {
    for (invoked, expected) in [(Some("plan"), "plan"), (None, "answer")] {
        let world = World::configured()?;
        let endpoint = Endpoint::answering(build_choice(0.9), 0.5);
        world.append_transcript(&[earlier()])?;
        route(&world, "build T3", &endpoint)?;
        if let Some(skill) = invoked {
            world.append_transcript(&[skill_call(skill)])?;
        }

        route(&world, "next", &endpoint)?;

        let rows = world.rows()?;
        let labels = of_kind(&rows, "label");
        assert_eq!(labels.len(), 1, "{invoked:?}: {rows:?}");
        assert_eq!(labels[0]["outcome"], json!(expected), "{invoked:?}");
    }
    Ok(())
}

#[test]
fn a_transcript_rewritten_before_the_next_prompt_leaves_the_decision_unlabelled() -> TestResult {
    let world = World::configured()?;
    let endpoint = Endpoint::answering(build_choice(0.9), 0.5);
    world.append_transcript(&[earlier(), earlier()])?;
    route(&world, "build T3", &endpoint)?;
    std::fs::write(world.transcript(), b"")?;
    world.append_transcript(&[skill_call("build")])?;

    route(&world, "next", &endpoint)?;

    let rows = world.rows()?;
    assert!(of_kind(&rows, "label").is_empty(), "compacted: {rows:?}");
    Ok(())
}

#[test]
fn a_decision_is_labelled_once() -> TestResult {
    let world = World::configured()?;
    let endpoint = Endpoint::answering(build_choice(0.9), 0.5);
    world.append_transcript(&[earlier()])?;
    route(&world, "build T3", &endpoint)?;

    route(&world, "second", &endpoint)?;
    route(&world, "third", &endpoint)?;

    let rows = world.rows()?;
    let decisions = choices(&rows);
    let labels = of_kind(&rows, "label");
    assert_eq!(
        labels.len(),
        2,
        "the first two decisions, once each: {rows:?}"
    );
    assert_eq!(labels[0]["id"], decisions[0]["id"]);
    assert_eq!(labels[1]["id"], decisions[1]["id"]);
    Ok(())
}

/// Keeps every request body and answers none.
struct Recorder(Mutex<Vec<String>>);

impl Transport for Recorder {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(String::from_utf8_lossy(&request.body).into_owned());
        Err(TransportError::Unreachable {
            reason: "recorded".to_owned(),
        })
    }
}

#[test]
fn in_enforce_an_answer_that_cannot_be_logged_is_not_injected_and_is_one_error_line() -> TestResult
{
    let world = World::enforcing(Point::D3)?;
    block_the_log(world.home())?;

    let output = route(
        &world,
        "build T3",
        &Endpoint::answering(build_choice(0.9), 0.5),
    )?;

    assert_eq!(context(&output), None, "{output}");
    let message = output["systemMessage"].as_str().unwrap_or_default();
    assert!(message.starts_with("ERROR route: "), "{output}");
    assert!(
        message.contains("the phase decision was not logged"),
        "{message}"
    );
    assert!(
        !world
            .home()
            .join("decisions")
            .join("route-pending.json")
            .exists(),
        "an unlogged decision is not kept for labelling"
    );
    Ok(())
}

#[test]
fn with_too_little_of_the_budget_left_clef_is_not_asked() -> TestResult {
    let world = World::enforcing(Point::D3)?;
    let recorder = Recorder(Mutex::new(Vec::new()));

    // The budget starts at 0 s and is next read at 3 s: 1.5 s left, under Clef's 2 s.
    let output = route_at(&world, "build T3", &recorder, &Stepping::by_ms(3_000))?;

    assert_eq!(context(&output), None, "{output}");
    let message = output["systemMessage"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("ERROR route: Clef was not asked"),
        "{output}"
    );
    let bodies = recorder
        .0
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    assert!(bodies.is_empty(), "nothing sent: {bodies:?}");
    assert!(world.rows()?.is_empty(), "nothing logged");
    Ok(())
}

#[test]
fn the_prompt_sent_to_clef_is_redacted() -> TestResult {
    let world = World::configured()?;
    let recorder = Recorder(Mutex::new(Vec::new()));

    route(&world, "deploy with TOKEN=hunter2 now", &recorder)?;

    let bodies = recorder
        .0
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(bodies.len(), 2, "both questions asked");
    for body in &bodies {
        assert!(!body.contains("hunter2"), "{body}");
        assert!(body.contains("TOKEN=<redacted>"), "{body}");
    }
    Ok(())
}

/// Each call waits, at most 10 s, for the other to arrive: only calls made side by side meet.
struct Meeting {
    arrived: Mutex<u32>,
    all_here: Condvar,
}

impl Meeting {
    /// Counts this call in and wakes any call already waiting; the lock stays held.
    fn arrive(&self) -> MutexGuard<'_, u32> {
        let mut arrived = self.arrived.lock().unwrap_or_else(PoisonError::into_inner);
        *arrived += 1;
        self.all_here.notify_all();
        arrived
    }
}

impl Transport for Meeting {
    fn send(&self, _request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        // The guard handed back is dropped at the end of this statement.
        let waited = self
            .all_here
            .wait_timeout_while(self.arrive(), Duration::from_secs(10), |count| *count < 2)
            .unwrap_or_else(PoisonError::into_inner)
            .1;
        let reason = if waited.timed_out() {
            "asked alone"
        } else {
            "met"
        };
        Err(TransportError::Unreachable {
            reason: reason.to_owned(),
        })
    }
}

/// The hook's whole Clef budget is one call's timeout: the two questions run side by side.
#[test]
fn both_questions_are_asked_concurrently() -> TestResult {
    let world = World::configured()?;
    let meeting = Meeting {
        arrived: Mutex::new(0),
        all_here: Condvar::new(),
    };

    route(&world, "build T3", &meeting)?;

    let rows = world.rows()?;
    let reasons: Vec<&Value> = rows.iter().map(|row| &row["error_reason"]).collect();
    assert_eq!(
        reasons,
        [
            &json!("Clef unreachable: met"),
            &json!("Clef unreachable: met")
        ],
        "both met"
    );
    Ok(())
}
