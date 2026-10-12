//! `anneal verify risk` with a fake Clef: D5 on the flash model, a skip only in effect enforce
//! with no trigger, and the review otherwise.

use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use super::{RiskArgs, VerifyAction, VerifyArgs, run};
use crate::decide::DecideEnv;
use crate::decide::point::Point;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::hook::clef_fake::{Fixed, TestResult, World, block_the_log, reply};
use crate::test_repo::ScratchRepo;

/// Answers D5's score with `score`, recording the model asked.
struct Endpoint {
    score: f64,
    models: Mutex<Vec<String>>,
}

impl Endpoint {
    fn scoring(score: f64) -> Self {
        Self {
            score,
            models: Mutex::new(Vec::new()),
        }
    }

    fn models(&self) -> Vec<String> {
        // The list is copied out of the lock for the assertion.
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Endpoint {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.model.to_owned());
        reply(Ok(json!({ "score": self.score })))
    }
}

/// A repository whose change from `main` writes `files`.
fn changed(files: &[(&str, &str)]) -> TestResult<ScratchRepo> {
    let repo = ScratchRepo::new()?;
    for (path, content) in files {
        repo.write(path, content)?;
    }
    Ok(repo)
}

/// Runs `verify risk --base main` in `repo`: the exit code and the envelope.
fn risk(world: &World, repo: &ScratchRepo, clef: &Endpoint) -> TestResult<(u8, Value)> {
    let args = VerifyArgs {
        action: VerifyAction::Risk(RiskArgs {
            base: Some("main".to_owned()),
        }),
    };
    let env = DecideEnv {
        cwd: repo.path(),
        lookup: |name: &str| world.lookup(name),
        transport: clef,
        clock: &Fixed,
    };
    let mut out = Vec::new();
    let code = run(&args, &env, &mut out)?;
    Ok((code.value(), serde_json::from_slice(&out)?))
}

fn d5_rows(world: &World) -> TestResult<Vec<Value>> {
    Ok(world
        .rows()?
        .into_iter()
        .filter(|row| row["point"] == json!("D5"))
        .collect())
}

#[test]
fn d5_asks_the_flash_model_about_the_change_and_keeps_no_input() -> TestResult {
    let world = World::configured()?;
    let repo = changed(&[("src/parser.rs", "fn parse() {}\n")])?;
    let clef = Endpoint::scoring(0.9);

    let (code, envelope) = risk(&world, &repo, &clef)?;

    assert_eq!(code, 0, "{envelope}");
    assert_eq!(clef.models(), vec!["@cf/wet-ink/clef-flash".to_owned()]);
    let rows = d5_rows(&world)?;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].get("input"), None, "the row keeps no paths");
    assert_eq!(envelope["data"]["clef"]["action"], json!("doubt_review"));
    Ok(())
}

#[test]
fn in_shadow_a_skip_answer_still_sends_the_change_to_review() -> TestResult {
    let world = World::configured()?;
    let repo = changed(&[("src/parser.rs", "fn parse() {}\n")])?;

    let (code, envelope) = risk(&world, &repo, &Endpoint::scoring(0.1))?;

    assert_eq!(code, 0, "shadow never skips: {envelope}");
    assert_eq!(envelope["data"]["doubt_review"], json!(true));
    assert_eq!(envelope["data"]["clef"]["action"], json!("skip"));
    assert_eq!(envelope["data"]["clef"]["enforce"], json!(false));
    Ok(())
}

#[test]
fn in_enforce_a_skip_with_no_trigger_exits_3() -> TestResult {
    let world = World::enforcing(Point::D5)?;
    let repo = changed(&[("src/parser.rs", "fn parse() {}\n")])?;

    let (code, envelope) = risk(&world, &repo, &Endpoint::scoring(0.1))?;

    assert_eq!(code, 3, "{envelope}");
    assert_eq!(envelope["data"]["doubt_review"], json!(false));
    assert_eq!(envelope["data"]["triggers"], json!([]));
    Ok(())
}

#[test]
fn in_enforce_a_trigger_overrides_a_skip() -> TestResult {
    let world = World::enforcing(Point::D5)?;
    let repo = changed(&[("src/auth.rs", "fn login() {}\n")])?;

    let (code, envelope) = risk(&world, &repo, &Endpoint::scoring(0.1))?;

    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["doubt_review"], json!(true));
    assert_eq!(
        envelope["data"]["triggers"],
        json!([{"trigger": "path", "path": "src/auth.rs", "glob": "**/*auth*", "category": "auth"}])
    );
    Ok(())
}

#[test]
fn in_enforce_an_answer_that_cannot_be_logged_is_not_acted_on() -> TestResult {
    let world = World::enforcing(Point::D5)?;
    block_the_log(world.home())?;
    let repo = changed(&[("src/parser.rs", "fn parse() {}\n")])?;

    let (code, envelope) = risk(&world, &repo, &Endpoint::scoring(0.1))?;

    assert_eq!(code, 0, "an unlogged skip still reviews: {envelope}");
    assert_eq!(envelope["data"]["clef"]["clef"], json!("unlogged"));
    Ok(())
}

#[test]
fn an_empty_change_asks_nothing() -> TestResult {
    let world = World::enforcing(Point::D5)?;
    let repo = changed(&[])?;
    let clef = Endpoint::scoring(0.1);

    let (code, envelope) = risk(&world, &repo, &clef)?;

    assert_eq!(code, 0, "{envelope}");
    assert!(clef.models().is_empty(), "nothing to ask about");
    assert_eq!(envelope["data"]["clef"]["clef"], json!("not_asked"));
    Ok(())
}

#[test]
fn with_clef_off_the_triggers_alone_decide_and_nothing_is_created() -> TestResult {
    let world = World::new(&[])?;
    let lines = "x\n".repeat(401);
    let repo = changed(&[("notes.txt", lines.as_str())])?;
    let clef = Endpoint::scoring(0.1);

    let (code, envelope) = risk(&world, &repo, &clef)?;

    assert_eq!(code, 0, "{envelope}");
    assert!(clef.models().is_empty(), "off asks nothing");
    assert_eq!(envelope["data"]["clef"]["clef"], json!("off"));
    assert_eq!(
        envelope["data"]["triggers"],
        json!([{"trigger": "lines", "lines": 401, "limit": 400}])
    );
    assert_eq!(
        std::fs::read_dir(world.home())?.count(),
        0,
        "no log created"
    );
    Ok(())
}
