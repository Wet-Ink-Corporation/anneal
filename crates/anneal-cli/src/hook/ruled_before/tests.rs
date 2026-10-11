//! `anneal hook ruled-before` with a fake Clef: a ruling is surfaced only in enforce mode
//! above the threshold, and the options are the rulings plus `none`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{precedents, run, state_for};
use crate::decide::clock::Clock;
use crate::decide::point::Point;
use crate::decide::state::CompactState;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::hook::HookEnv;
use crate::hook::clef_fake::{
    Fixed, Stepping, TestResult, World, block_the_log, context, parsed, reply,
};
use crate::hook::prompt::safe_prompt;

/// Answers every choice with one fixed reply and keeps each request's body.
struct Endpoint {
    answer: Result<Value, TransportError>,
    bodies: Mutex<Vec<String>>,
}

impl Endpoint {
    fn answering(answer: Value) -> Self {
        Self {
            answer: Ok(answer),
            bodies: Mutex::new(Vec::new()),
        }
    }

    fn bodies(&self) -> Vec<String> {
        // The test reads its own copy of what was sent.
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Endpoint {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(String::from_utf8_lossy(&request.body).into_owned());
        // Each call hands out its own copy of the fixed reply.
        reply(self.answer.clone())
    }
}

/// A repository directory with a ruling and an accepted weigh-in atom.
fn project() -> TestResult<TempDir> {
    let root = tempfile::tempdir()?;
    let store = root.path().join(".out-of-scope");
    fs::create_dir_all(&store)?;
    fs::write(
        store.join("no-gui.md"),
        "---\nid: no-gui\nruled_at: 2026-10-11\nsummary: No GUI in v0.1; the CLI is the surface.\n---\n",
    )?;
    let atoms = root.path().join(".kb").join("_intake").join("decisions");
    fs::create_dir_all(&atoms)?;
    fs::write(
        atoms.join("wi-a53271-which-licence.md"),
        "---\nid: \"kb-decision-wi-a53271\"\ntitle: \"Which licence should anneal ship under: \
         MIT OR Apache-2.0\"\nstatus: accepted\nweighin_item: \"wi-a53271\"\n\
         question: \"Which licence should anneal ship under?\"\n---\n",
    )?;
    Ok(root)
}

/// Runs the hook on `prompt` in `cwd`, returning its stdout parsed (`Null` when silent).
fn ruled_before(
    world: &World,
    cwd: &Path,
    prompt: &str,
    transport: &impl Transport,
) -> TestResult<Value> {
    ruled_before_at(world, (cwd, prompt), transport, &Fixed)
}

/// [`ruled_before`] with `clock`.
fn ruled_before_at(
    world: &World,
    (cwd, prompt): (&Path, &str),
    transport: &impl Transport,
    clock: &impl Clock,
) -> TestResult<Value> {
    let stdin = world.input(prompt, cwd);
    let env = HookEnv {
        stdin: stdin.as_slice(),
        current_dir: || Ok(PathBuf::new()),
        lookup: |name: &str| world.lookup(name),
    };
    let mut out = Vec::new();
    run(env, transport, clock, &mut out)?;
    parsed(&out)
}

fn licence_choice(top: f64) -> Value {
    json!({"probabilities": {"wi-a53271": top, "none": 1.0 - top, "no-gui": 0.0}})
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_weigh_in_atom_is_an_option_with_its_wi_id_beside_the_rulings_and_none() -> TestResult {
    let root = project()?;
    let world = World::configured()?;
    let endpoint = Endpoint::answering(licence_choice(0.9));

    ruled_before(&world, root.path(), "let us switch to GPL", &endpoint)?;

    let bodies = endpoint.bodies();
    let [body] = bodies.as_slice() else {
        return Err(format!("one call: {bodies:?}").into());
    };
    let request: Value = serde_json::from_str(body)?;
    let question: Value = serde_json::from_str(
        request
            .pointer("/messages/0/content")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )?;
    assert_eq!(question["options"], json!(["no-gui", "wi-a53271", "none"]));
    assert_eq!(question["subject"], json!("ruled_before"));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn in_enforce_a_ruling_over_the_threshold_is_surfaced_in_one_line() -> TestResult {
    let root = project()?;
    let world = World::enforcing(Point::D6)?;

    let output = ruled_before(
        &world,
        root.path(),
        "let us switch to GPL",
        &Endpoint::answering(licence_choice(0.9)),
    )?;

    assert_eq!(
        context(&output),
        Some(
            "anneal ruled-before: this was ruled on already; do not re-open it unless the \
             human asks to: wi-a53271 (Which licence should anneal ship under: MIT OR \
             Apache-2.0)."
        ),
        "{output}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn in_shadow_a_ruling_over_the_threshold_is_logged_but_not_surfaced() -> TestResult {
    let root = project()?;
    let world = World::configured()?;

    let output = ruled_before(
        &world,
        root.path(),
        "let us switch to GPL",
        &Endpoint::answering(licence_choice(0.9)),
    )?;

    assert_eq!(output, Value::Null);
    let rows = world.rows()?;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["point"], json!("D6"));
    assert_eq!(rows[0]["action"], json!("surface"));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn in_enforce_none_or_an_answer_under_the_threshold_surfaces_nothing() -> TestResult {
    let root = project()?;
    let world = World::enforcing(Point::D6)?;
    let none = json!({"probabilities": {"none": 0.95, "wi-a53271": 0.05, "no-gui": 0.0}});

    for answer in [none, licence_choice(0.6)] {
        let output = ruled_before(
            &world,
            root.path(),
            "add a flag",
            &Endpoint::answering(answer),
        )?;

        assert_eq!(output, Value::Null);
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn with_no_rulings_nothing_is_asked() -> TestResult {
    let root = tempfile::tempdir()?;
    let world = World::configured()?;
    let endpoint = Endpoint::answering(licence_choice(0.9));

    let output = ruled_before(&world, root.path(), "add a flag", &endpoint)?;

    assert_eq!(output, Value::Null);
    assert!(endpoint.bodies().is_empty(), "nothing to choose from");
    assert!(world.rows()?.is_empty(), "nothing logged");
    Ok(())
}

#[test]
fn with_clef_off_nothing_is_read_asked_or_written() -> TestResult {
    let root = project()?;
    let world = World::new(&[])?;
    let endpoint = Endpoint::answering(licence_choice(0.9));

    let output = ruled_before(&world, root.path(), "switch to GPL", &endpoint)?;

    assert_eq!(output, Value::Null);
    assert!(endpoint.bodies().is_empty(), "nothing asked");
    assert_eq!(fs::read_dir(world.home())?.count(), 0, "nothing created");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn an_invalid_ruling_is_one_error_line_and_nothing_is_asked() -> TestResult {
    let root = project()?;
    fs::write(
        root.path().join(".out-of-scope").join("bad.md"),
        "no frontmatter\n",
    )?;
    let world = World::configured()?;
    let endpoint = Endpoint::answering(licence_choice(0.9));

    let output = ruled_before(&world, root.path(), "switch to GPL", &endpoint)?;

    assert_eq!(context(&output), None);
    let message = output["systemMessage"].as_str().unwrap_or_default();
    assert!(message.starts_with("ERROR ruled-before: "), "{output}");
    assert!(message.contains("bad.md"), "names the file: {message}");
    assert!(endpoint.bodies().is_empty(), "nothing asked");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn an_unreachable_clef_surfaces_nothing_and_logs_the_miss() -> TestResult {
    let root = project()?;
    let world = World::enforcing(Point::D6)?;
    let endpoint = Endpoint {
        answer: Err(TransportError::Unreachable {
            reason: "no HTTP client".to_owned(),
        }),
        bodies: Mutex::new(Vec::new()),
    };

    let output = ruled_before(&world, root.path(), "switch to GPL", &endpoint)?;

    assert_eq!(output, Value::Null, "fails open, silently");
    assert_eq!(world.rows()?[0]["unreachable"], json!(true));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn in_enforce_a_ruling_whose_row_cannot_be_logged_is_not_surfaced_and_is_one_error_line()
-> TestResult {
    let root = project()?;
    let world = World::enforcing(Point::D6)?;
    block_the_log(world.home())?;

    let output = ruled_before(
        &world,
        root.path(),
        "let us switch to GPL",
        &Endpoint::answering(licence_choice(0.9)),
    )?;

    assert_eq!(context(&output), None, "{output}");
    let message = output["systemMessage"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("ERROR ruled-before: the decision was not logged"),
        "{output}"
    );
    Ok(())
}

#[test]
fn with_too_little_of_the_budget_left_neither_git_nor_clef_is_started() -> TestResult {
    let root = project()?;
    let world = World::enforcing(Point::D6)?;
    let endpoint = Endpoint::answering(licence_choice(0.9));

    // The budget starts at 0 s and is next read at 3 s: 1.5 s left, under Clef's 2 s.
    let output = ruled_before_at(
        &world,
        (root.path(), "let us switch to GPL"),
        &endpoint,
        &Stepping::by_ms(3_000),
    )?;

    assert_eq!(context(&output), None, "{output}");
    let message = output["systemMessage"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("ERROR ruled-before: Clef was not asked"),
        "{output}"
    );
    assert!(endpoint.bodies().is_empty(), "nothing asked");
    assert!(world.rows()?.is_empty(), "nothing logged");
    Ok(())
}

#[test]
fn the_largest_state_the_hook_builds_fits_the_compact_state() -> TestResult {
    let root = tempfile::tempdir()?;
    let atoms = root.path().join(".kb").join("_intake").join("decisions");
    fs::create_dir_all(&atoms)?;
    // Backslashes double in JSON: the longest summary the cap lets through.
    let title = "\\".repeat(400);
    for n in 0..40 {
        let id = format!("wi-{n:0>61}");
        fs::write(
            atoms.join(format!("{id}.md")),
            format!("---\ntitle: {title}\nstatus: accepted\nweighin_item: \"{id}\"\n---\n"),
        )?;
    }
    let found = precedents::gather(root.path())?;
    // Four bytes a character, the most UTF-8 spends.
    let prompt = safe_prompt(&"\u{1F600}".repeat(5_000));

    let state = CompactState::parse(&state_for(&prompt, &found).to_string());

    assert_eq!(found.len(), precedents::MAX_PRECEDENTS);
    assert!(
        found
            .iter()
            .all(|p| p.id().len() == 64 && p.summary().len() == precedents::MAX_SUMMARY_BYTES),
        "{found:?}"
    );
    assert!(state.is_ok(), "{state:?}");
    Ok(())
}
