//! `anneal grill classify` with a fake Clef: shadow only logs, enforce writes `is_decision`,
//! and nothing is asked for a question that is not an open node of a valid tree.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use clap::Parser as _;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::cli::{Cli, Command};
use crate::decide::DecideEnv;
use crate::decide::point::Point;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::error::CliError;
use crate::grill::{GrillAction, run};
use crate::hook::clef_fake::{CONFIGURED, Fixed, TestResult, World, block_the_log, reply};

/// The variable that sets Clef's mode.
const MODE: &str = "CLAUDE_PLUGIN_OPTION_CLEF_MODE";

/// Answers every request with one fixed reply and keeps each request body.
struct Endpoint {
    answer: Result<Value, TransportError>,
    bodies: Mutex<Vec<String>>,
}

impl Endpoint {
    /// Answers `yes` with probability `yes`.
    fn yes(yes: f64) -> Self {
        Self::replying(Ok(json!({"probabilities": {"yes": yes, "no": 1.0 - yes}})))
    }

    fn unreachable() -> Self {
        Self::replying(Err(TransportError::Unreachable {
            reason: "no route".to_owned(),
        }))
    }

    fn replying(answer: Result<Value, TransportError>) -> Self {
        Self {
            answer,
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

/// A node as JSON, open unless `status` says otherwise.
fn node(id: &str, status: &str, is_decision: f64) -> Value {
    json!({
        "id": id,
        "question": format!("Which {id} do we use?"),
        "depends_on": [],
        "status": status,
        "answer": format!("the usual {id}"),
        "is_decision": is_decision,
        "confidence": 0.5,
    })
}

/// A project holding `tree.json` with `nodes` (in this order); returns it and the bytes.
fn project(nodes: &[Value]) -> TestResult<(TempDir, Vec<u8>)> {
    let dir = tempfile::tempdir()?;
    let bytes = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "goal": "export orders as CSV",
        "turns": [],
        "nodes": nodes,
    }))?;
    std::fs::write(dir.path().join("tree.json"), &bytes)?;
    Ok((dir, bytes))
}

/// The usual project: `scope` answered, then `library` (a fact, 0.2) and `format` open.
fn usual() -> TestResult<(TempDir, Vec<u8>)> {
    project(&[
        node("scope", "answered", 0.9),
        node("library", "open", 0.2),
        node("format", "open", 0.8),
    ])
}

/// What one `anneal grill classify` run gave: its exit code and envelope, or its error.
struct Ran {
    result: Result<(u8, Value), CliError>,
}

impl Ran {
    fn code(&self) -> Option<u8> {
        self.result.as_ref().ok().map(|(code, _)| *code)
    }

    fn data(&self) -> TestResult<&Value> {
        match &self.result {
            Ok((_, envelope)) => Ok(&envelope["data"]),
            Err(error) => Err(format!("classify failed: {error:?}").into()),
        }
    }
}

/// Runs `anneal grill classify --tree tree.json --node <node>` in `cwd` with `world`'s
/// variables and `endpoint` as Clef.
fn classify(world: &World, cwd: &Path, node: &str, endpoint: &Endpoint) -> TestResult<Ran> {
    let parsed = Cli::try_parse_from([
        "anneal",
        "grill",
        "classify",
        "--tree",
        "tree.json",
        "--node",
        node,
    ])?;
    let Some(Command::Grill(args)) = parsed.command else {
        return Err("not grill".into());
    };
    if !matches!(args.action, GrillAction::Classify(_)) {
        return Err("not classify".into());
    }
    let env = DecideEnv {
        cwd,
        lookup: |name: &str| world.lookup(name),
        transport: endpoint,
        clock: &Fixed,
    };
    let mut out = Vec::new();
    let result = match run(&args, &env, &mut out) {
        Ok(code) => Ok((code.value(), serde_json::from_slice(&out)?)),
        Err(error) => Err(error),
    };
    Ok(Ran { result })
}

fn tree_bytes(dir: &Path) -> TestResult<Vec<u8>> {
    Ok(std::fs::read(dir.join("tree.json"))?)
}

#[test]
fn with_clef_off_nothing_is_asked_logged_or_written() -> TestResult {
    let world = World::new(&[])?;
    let (dir, before) = usual()?;
    let endpoint = Endpoint::yes(0.9);

    let ran = classify(&world, dir.path(), "library", &endpoint)?;

    assert_eq!(ran.code(), Some(0), "Clef off is not a failure for grill");
    let data = ran.data()?;
    assert_eq!(
        (&data["clef"], &data["written"]),
        (&json!("off"), &json!(false))
    );
    assert!(endpoint.bodies().is_empty(), "Clef is not asked");
    assert_eq!(
        std::fs::read_dir(world.home())?.count(),
        0,
        "nothing logged"
    );
    assert_eq!(tree_bytes(dir.path())?, before, "the tree is untouched");
    Ok(())
}

#[test]
fn in_shadow_the_answer_is_logged_and_the_tree_left_as_the_model_wrote_it() -> TestResult {
    let world = World::configured()?;
    let (dir, before) = usual()?;

    let ran = classify(&world, dir.path(), "library", &Endpoint::yes(0.9))?;

    assert_eq!(ran.code(), Some(0));
    let data = ran.data()?;
    assert_eq!(
        [
            &data["clef"],
            &data["node"],
            &data["is_decision"],
            &data["action"],
            &data["mode"],
            &data["written"],
        ],
        [
            &json!("decided"),
            &json!("library"),
            &json!(0.9),
            &json!("ask_human"),
            &json!("shadow"),
            &json!(false),
        ]
    );
    let rows = world.rows()?;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = rows.first().ok_or("a row")?;
    assert_eq!(
        [&row["point"], &row["question"], &row["subject"], &row["id"]],
        [
            &json!("D7"),
            &json!("noul"),
            &json!("is_decision"),
            &data["id"]
        ]
    );
    assert_eq!(tree_bytes(dir.path())?, before, "shadow writes nothing");
    Ok(())
}

#[test]
fn in_enforce_clefs_probability_is_written_into_that_node_only() -> TestResult {
    let world = World::enforcing(Point::D7)?;
    let (dir, _) = usual()?;

    let ran = classify(&world, dir.path(), "format", &Endpoint::yes(0.25))?;

    assert_eq!(ran.code(), Some(0));
    let data = ran.data()?;
    assert_eq!(
        [&data["action"], &data["mode"], &data["written"]],
        [&json!("find_fact"), &json!("enforce"), &json!(true)]
    );
    let written: Value = serde_json::from_slice(&tree_bytes(dir.path())?)?;
    let expected = [
        node("scope", "answered", 0.9),
        node("library", "open", 0.2),
        node("format", "open", 0.25),
    ];
    assert_eq!(
        written["nodes"],
        json!(expected),
        "only format's is_decision changes; the nodes keep the file's order"
    );
    assert_eq!(written["goal"], json!("export orders as CSV"));
    Ok(())
}

#[test]
fn an_unreachable_clef_is_logged_and_falls_back_to_the_models_estimate() -> TestResult {
    let world = World::enforcing(Point::D7)?;
    let (dir, before) = usual()?;

    let ran = classify(&world, dir.path(), "format", &Endpoint::unreachable())?;

    assert_eq!(ran.code(), Some(0), "D7 fails open");
    let data = ran.data()?;
    assert_eq!(
        [&data["clef"], &data["fallback"], &data["written"]],
        [&json!("no_answer"), &json!("fail_open"), &json!(false)]
    );
    let rows = world.rows()?;
    assert_eq!(
        rows.iter()
            .map(|row| &row["unreachable"])
            .collect::<Vec<_>>(),
        [&json!(true)]
    );
    assert_eq!(tree_bytes(dir.path())?, before);
    Ok(())
}

#[test]
fn an_answer_that_cannot_be_logged_is_an_error_and_never_written() -> TestResult {
    let world = World::enforcing(Point::D7)?;
    block_the_log(world.home())?;
    let (dir, before) = usual()?;

    let ran = classify(&world, dir.path(), "format", &Endpoint::yes(0.1))?;

    let code = ran.result.as_ref().err().map(|error| error.code().value());
    assert_eq!(code, Some(70), "{:?}", ran.result);
    assert_eq!(tree_bytes(dir.path())?, before);
    Ok(())
}

#[test]
fn a_node_that_is_missing_or_not_open_is_a_usage_error_and_asks_nothing() -> TestResult {
    let world = World::configured()?;
    let (dir, _) = usual()?;
    let endpoint = Endpoint::yes(0.9);

    let codes: Vec<Option<u8>> = ["nope", "scope"]
        .into_iter()
        .map(|id| {
            classify(&world, dir.path(), id, &endpoint)
                .map(|ran| ran.result.err().map(|error| error.code().value()))
        })
        .collect::<TestResult<_>>()?;

    assert_eq!(codes, [Some(2), Some(2)]);
    assert!(endpoint.bodies().is_empty());
    assert_eq!(world.rows()?, Vec::<Value>::new());
    Ok(())
}

#[test]
fn an_invalid_tree_exits_1_and_asks_nothing() -> TestResult {
    let world = World::configured()?;
    let (dir, _) = project(&[node("a", "open", 1.5)])?;
    let endpoint = Endpoint::yes(0.9);

    let ran = classify(&world, dir.path(), "a", &endpoint)?;

    assert_eq!(ran.code(), Some(1));
    assert!(endpoint.bodies().is_empty());
    assert_eq!(world.rows()?, Vec::<Value>::new());
    Ok(())
}

#[test]
fn clef_gets_the_redacted_question_and_goal_and_the_row_keeps_neither() -> TestResult {
    let world = World::configured()?;
    let mut secret = node("auth", "open", 0.7);
    secret["question"] = json!("Do we keep   API_KEY=abc123 in the vault?");
    let (dir, _) = project(&[secret])?;
    let endpoint = Endpoint::yes(0.9);

    classify(&world, dir.path(), "auth", &endpoint)?;

    let bodies = endpoint.bodies();
    let body = bodies.first().ok_or("one request")?;
    assert!(
        body.contains("Do we keep API_KEY=<redacted> in the vault?"),
        "{body}"
    );
    assert!(body.contains("export orders as CSV"), "{body}");
    assert!(!body.contains("abc123"), "{body}");
    let rows = world.rows()?;
    let row = rows.first().ok_or("a row")?;
    assert_eq!(row.get("input"), None, "{row}");
    assert!(!row.to_string().contains("vault"), "{row}");
    Ok(())
}

#[test]
fn enforce_within_its_first_50_answers_is_shadow_and_writes_nothing() -> TestResult {
    let mut vars = CONFIGURED.to_vec();
    vars.push((MODE, "enforce"));
    let world = World::new(&vars)?;
    let (dir, before) = usual()?;

    let ran = classify(&world, dir.path(), "format", &Endpoint::yes(0.25))?;

    assert_eq!(ran.code(), Some(0));
    let data = ran.data()?;
    assert_eq!(
        [&data["clef"], &data["mode"], &data["written"]],
        [&json!("decided"), &json!("shadow"), &json!(false)]
    );
    let rows = world.rows()?;
    let row = rows.first().ok_or("a row")?;
    assert_eq!(
        [&row["mode"], &row["configured_mode"]],
        [&json!("shadow"), &json!("enforce")],
        "configured to enforce, but still in its shadow period"
    );
    assert_eq!(
        tree_bytes(dir.path())?,
        before,
        "the shadow period writes nothing"
    );
    Ok(())
}

#[test]
fn a_malformed_configuration_falls_back_and_asks_logs_and_writes_nothing() -> TestResult {
    let mut vars = CONFIGURED.to_vec();
    vars.push((MODE, "sometimes"));
    let world = World::new(&vars)?;
    let (dir, before) = usual()?;
    let endpoint = Endpoint::yes(0.9);

    let ran = classify(&world, dir.path(), "format", &endpoint)?;

    assert_eq!(ran.code(), Some(0), "D7 fails open");
    let data = ran.data()?;
    assert_eq!(
        [&data["clef"], &data["fallback"], &data["written"]],
        [&json!("misconfigured"), &json!("fail_open"), &json!(false)]
    );
    assert!(endpoint.bodies().is_empty(), "Clef is not asked");
    assert_eq!(world.rows()?, Vec::<Value>::new(), "nothing logged");
    assert_eq!(tree_bytes(dir.path())?, before);
    Ok(())
}

#[test]
fn thresholds_that_cannot_be_read_fall_back_instead_of_failing() -> TestResult {
    let plugin = tempfile::tempdir()?;
    let root = plugin.path().to_str().ok_or("a UTF-8 temporary path")?;
    let mut vars = CONFIGURED.to_vec();
    vars.push(("CLAUDE_PLUGIN_ROOT", root));
    let world = World::new(&vars)?;
    let (dir, before) = usual()?;
    let endpoint = Endpoint::yes(0.9);

    let ran = classify(&world, dir.path(), "format", &endpoint)?;

    assert_eq!(ran.code(), Some(0), "D7 fails open: {:?}", ran.result);
    let data = ran.data()?;
    assert_eq!(
        [&data["clef"], &data["fallback"], &data["written"]],
        [&json!("misconfigured"), &json!("fail_open"), &json!(false)]
    );
    let why = data["reason"].as_str().ok_or("a reason")?;
    assert!(why.contains("thresholds"), "{why}");
    assert!(endpoint.bodies().is_empty(), "Clef is not asked");
    assert_eq!(tree_bytes(dir.path())?, before);
    Ok(())
}

#[cfg(unix)]
#[test]
fn an_enforced_estimate_is_written_through_a_symlinked_tree_which_stays_a_link() -> TestResult {
    let world = World::enforcing(Point::D7)?;
    let (dir, _) = usual()?;
    let real = dir.path().join("real.json");
    std::fs::rename(dir.path().join("tree.json"), &real)?;
    std::os::unix::fs::symlink("real.json", dir.path().join("tree.json"))?;

    let ran = classify(&world, dir.path(), "format", &Endpoint::yes(0.25))?;

    assert_eq!(ran.code(), Some(0));
    assert_eq!(ran.data()?["written"], json!(true));
    let link = std::fs::symlink_metadata(dir.path().join("tree.json"))?;
    assert!(link.file_type().is_symlink(), "the link is kept");
    let written: Value = serde_json::from_slice(&std::fs::read(&real)?)?;
    assert_eq!(
        written["nodes"][2]["is_decision"],
        json!(0.25),
        "written through it"
    );
    Ok(())
}
