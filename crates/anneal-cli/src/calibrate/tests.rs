//! `anneal calibrate` through `cli`'s argument parsing, with an anneal home of its own.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTimeError};

use clap::Parser;
use serde_json::{Value, json};

use super::{CalibrateArgs, CalibrateEnv, run};
use crate::cli::{Cli, Command};
use crate::decide::{
    self, DecideEnv,
    clock::Clock,
    transport::{ClefRequest, Transport, TransportError},
};
use crate::error::CliError;
use crate::exit_codes::Code;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn args(words: &[&str]) -> CalibrateArgs {
    let parsed = Cli::try_parse_from(["anneal", "calibrate"].iter().chain(words))
        .expect("the arguments parse");
    match parsed.command {
        Some(Command::Calibrate(args)) => args,
        other => panic!("not calibrate: {other:?}"),
    }
}

/// What one run wrote: the exit code, the envelope and stderr.
struct Ran {
    code: Result<u8, CliError>,
    envelope: Value,
    stderr: String,
}

/// Runs `anneal calibrate <words>` with `home` as the anneal home and no plugin root.
fn calibrate(home: &Path, words: &[&str]) -> Ran {
    let mut err = Vec::new();
    let (code, envelope) = calibrate_into(home, words, &mut err);
    Ran {
        code,
        envelope,
        stderr: String::from_utf8(err).expect("the summary is UTF-8"),
    }
}

/// As [`calibrate`], with stderr going to `err`; returns the exit code and the envelope.
fn calibrate_into<E: Write>(
    home: &Path,
    words: &[&str],
    err: &mut E,
) -> (Result<u8, CliError>, Value) {
    let env = CalibrateEnv {
        cwd: home,
        // The lookup hands out an owned value per call, as `std::env::var_os` does.
        lookup: |var: &str| -> Option<OsString> {
            (var == "ANNEAL_HOME").then(|| home.as_os_str().to_owned())
        },
    };
    let mut out = Vec::new();
    let code = run(&args(words), &env, &mut out, err).map(Code::value);
    let envelope = if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).expect("the envelope is JSON")
    };
    (code, envelope)
}

/// A stderr that refuses every write and flush.
struct ClosedStderr;

impl Write for ClosedStderr {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// A stderr that remembers whether it was flushed after its last write.
#[derive(Default)]
struct FlushedStderr {
    bytes: Vec<u8>,
    flushed: bool,
}

impl Write for FlushedStderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.flushed = false;
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

fn decision(id: &str, point: &str, probability: f64, action: &str) -> Value {
    json!({
        "kind": "decision", "id": id, "ts_ms": 1, "point": point, "question": "noul",
        "subject": "s", "model": "m", "state_sha256": "sha256:x",
        "probabilities": {"yes": probability, "no": 1.0 - probability}, "top": null,
        "probability": probability, "action": action, "mode": "shadow",
        "configured_mode": "shadow", "latency_ms": 1, "outcome": null, "unreachable": false,
        "error": null, "error_reason": null,
    })
}

fn label(id: &str, outcome: &str) -> Value {
    json!({"kind": "label", "id": id, "outcome": outcome, "ts_ms": 2})
}

fn write_log(home: &Path, file: &str, rows: &[Value]) -> io::Result<PathBuf> {
    let dir = home.join("decisions");
    fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    let text: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    fs::write(&path, text)?;
    Ok(path)
}

/// The hand set as D1 decisions, labelled.
fn hand_log(home: &Path) -> io::Result<()> {
    let set = [
        (0.1, "defer", "no"),
        (0.3, "defer", "yes"),
        (0.8, "ask", "yes"),
        (0.95, "deny", "no"),
        (1.0, "deny", "yes"),
    ];
    let mut rows = Vec::new();
    for (k, (probability, action, outcome)) in set.into_iter().enumerate() {
        let id = format!("d1-1-{k}");
        rows.push(decision(&id, "D1", probability, action));
        rows.push(label(&id, outcome));
    }
    write_log(home, "decisions.jsonl", &rows).map(|_| ())
}

/// `n` D7 decisions labelled exactly as predicted, none asking.
fn perfect_d7(home: &Path, n: u32) -> io::Result<()> {
    let mut rows = Vec::new();
    for k in 0..n {
        let id = format!("d1-7-{k}");
        let positive = k % 2 == 0;
        let probability = if positive { 1.0 } else { 0.0 };
        rows.push(decision(&id, "D7", probability, "find_fact"));
        rows.push(label(&id, if positive { "yes" } else { "no" }));
    }
    write_log(home, "decisions.jsonl", &rows).map(|_| ())
}

fn point<'v>(envelope: &'v Value, name: &str) -> &'v Value {
    envelope["data"]["points"]
        .as_array()
        .expect("points")
        .iter()
        .find(|point| point["point"] == json!(name))
        .expect("the point is reported")
}

#[test]
fn the_envelope_reports_each_point_and_the_proposal_to_apply_by_hand() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let ran = calibrate(home.path(), &[]);

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    assert_eq!(ran.envelope["ok"], json!(true));
    assert_eq!(ran.envelope["command"], json!("calibrate"));
    assert_eq!(
        ran.envelope["data"]["points"].as_array().map(Vec::len),
        Some(7)
    );
    let noul = &point(&ran.envelope, "D1")["questions"][0];
    assert_eq!(
        (&noul["n"], &noul["labelled"], &noul["verdict"]),
        (&json!(5), &json!(5), &json!("stay-shadow"))
    );
    assert_eq!(noul["threshold"]["proposed"], json!(0.3));
    assert_eq!(
        ran.envelope["data"]["apply"],
        json!([{
            "kind": "threshold", "point": "D1", "question": "noul", "file": null,
            "pointer": "/points/D1/rules/noul/bands/1/at_least", "from": 0.4, "to": 0.3,
        }]),
        "built-in thresholds: no file to name"
    );
    Ok(())
}

#[test]
fn the_proposals_file_holds_every_point_even_when_one_is_selected() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let ran = calibrate(home.path(), &["--point", "D7"]);

    let proposals = home.path().join("decisions").join("proposals.json");
    assert_eq!(
        ran.envelope["data"]["proposals"],
        json!(proposals.to_str().expect("a UTF-8 temporary path"))
    );
    let points: Vec<&Value> = ran.envelope["data"]["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|point| &point["point"])
        .collect();
    assert_eq!(points, [&json!("D7")], "only the point asked for");
    assert_eq!(
        ran.envelope["data"]["apply"],
        json!([]),
        "D1's step is filtered out"
    );
    let file: Value = serde_json::from_slice(&fs::read(&proposals)?)?;
    assert_eq!(file["kind"], json!("anneal-threshold-proposals"));
    assert_eq!(file["points"].as_array().map(Vec::len), Some(7));
    assert_eq!(
        file["apply"].as_array().map(Vec::len),
        Some(1),
        "D1's step is kept"
    );
    Ok(())
}

#[test]
fn an_eligible_point_gets_a_mode_step_and_nothing_is_applied() -> TestResult {
    let home = tempfile::tempdir()?;
    perfect_d7(home.path(), 50)?;
    let thresholds = home.path().join("clef-thresholds.json");
    fs::write(&thresholds, decide::thresholds::BUILT_IN)?;
    let before = fs::read(&thresholds)?;

    let ran = calibrate(
        home.path(),
        &["--thresholds", "clef-thresholds.json", "--point", "D7"],
    );

    assert_eq!(
        point(&ran.envelope, "D7")["verdict"],
        json!("eligible-to-enforce")
    );
    let apply = ran.envelope["data"]["apply"].as_array().expect("apply");
    assert!(
        apply.contains(&json!({
            "kind": "mode", "point": "D7", "user_config": "clef_mode",
            "env": "CLAUDE_PLUGIN_OPTION_CLEF_MODE", "value": "D7=enforce",
        })),
        "{apply:?}"
    );
    assert_eq!(
        fs::read(&thresholds)?,
        before,
        "the thresholds file is never written"
    );
    Ok(())
}

#[test]
fn forty_nine_labelled_decisions_are_not_eligible() -> TestResult {
    let home = tempfile::tempdir()?;
    perfect_d7(home.path(), 49)?;

    let ran = calibrate(home.path(), &["--point", "D7"]);

    let d7 = point(&ran.envelope, "D7");
    assert_eq!(d7["verdict"], json!("stay-shadow"));
    assert_eq!(d7["questions"][0]["reasons"], json!(["too_few_labelled"]));
    let apply = ran.envelope["data"]["apply"].as_array().expect("apply");
    assert!(
        apply.iter().all(|step| step["kind"] != json!("mode")),
        "no mode step: {apply:?}"
    );
    Ok(())
}

#[test]
fn the_summary_on_stderr_names_each_verdict_and_where_the_proposal_is() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let ran = calibrate(home.path(), &["--point", "D1"]);

    let proposals = home.path().join("decisions").join("proposals.json");
    let lines: Vec<&str> = ran.stderr.lines().collect();
    assert!(lines.len() <= 8, "short: {lines:?}");
    assert!(lines.contains(&"D1: stay-shadow"), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("threshold 0.4 -> 0.3")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&*proposals.to_string_lossy())),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("apply by hand:")
            && line.contains("/points/D1/rules/noul/bands/1/at_least")),
        "{lines:?}"
    );
    Ok(())
}

#[test]
fn a_stderr_that_refuses_the_summary_still_exits_0_after_the_ok_envelope() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let (code, envelope) = calibrate_into(home.path(), &[], &mut ClosedStderr);

    assert_eq!(code.as_ref().ok(), Some(&0), "{code:?}");
    assert_eq!(envelope["ok"], json!(true), "the one envelope stands");
    Ok(())
}

#[test]
fn the_summary_is_flushed() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    let mut err = FlushedStderr::default();

    let (code, _envelope) = calibrate_into(home.path(), &[], &mut err);

    assert_eq!(code.as_ref().ok(), Some(&0), "{code:?}");
    assert!(!err.bytes.is_empty(), "a summary is written");
    assert!(err.flushed, "and flushed after its last write");
    Ok(())
}

/// Answers every Clef request with `{"probabilities": {"yes": 0.95, "no": 0.05}}`.
struct Answers;

impl Transport for Answers {
    fn send(&self, _request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        let inner = json!({"probabilities": {"yes": 0.95, "no": 0.05}});
        Ok(json!({"result": {"response": inner.to_string()}})
            .to_string()
            .into_bytes())
    }
}

struct Fixed;

impl Clock for Fixed {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        Ok(1_760_000_000_000)
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// Runs `anneal decide <words>` through decide's own write path, with `home` as the anneal
/// home and Clef configured; returns the envelope.
fn decide_in(home: &Path, words: &[&str]) -> Result<Value, Box<dyn std::error::Error>> {
    let parsed = Cli::try_parse_from(["anneal", "decide"].iter().chain(words))?;
    let Some(Command::Decide(decide_args)) = parsed.command else {
        return Err("not decide".into());
    };
    let env = DecideEnv {
        cwd: home,
        lookup: |var: &str| -> Option<OsString> {
            match var {
                "ANNEAL_HOME" => Some(home.as_os_str().to_owned()),
                "CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID" => Some("acct0123".into()),
                "CLAUDE_PLUGIN_OPTION_CF_API_TOKEN" => Some("token-value".into()),
                _ => None,
            }
        },
        transport: &Answers,
        clock: &Fixed,
    };
    let mut out = Vec::new();
    let code = decide::run(&decide_args, &env, &mut out)?;
    assert_eq!(code.value(), 0, "decide succeeded");
    Ok(serde_json::from_slice(&out)?)
}

#[test]
fn a_decision_and_label_written_by_decide_are_calibrated() -> TestResult {
    let home = tempfile::tempdir()?;
    let decided = decide_in(
        home.path(),
        &[
            "--point",
            "D1",
            "--question",
            "noul",
            "--state",
            r#"{"tool":"Bash"}"#,
        ],
    )?;
    let id = decided["data"]["id"]
        .as_str()
        .ok_or("decide reports an id")?;
    decide_in(home.path(), &["label", id, "yes"])?;

    let ran = calibrate(home.path(), &["--point", "D1"]);

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    assert_eq!(
        ran.envelope["data"]["skipped_rows"],
        json!(0),
        "decide's rows read"
    );
    let noul = &point(&ran.envelope, "D1")["questions"][0];
    assert_eq!((&noul["n"], &noul["labelled"]), (&json!(1), &json!(1)));
    let brier = noul["brier"].as_f64().ok_or("a Brier score")?;
    // One decision at 0.95, labelled yes: (0.95 - 1)^2.
    assert!((brier - 0.0025).abs() < 1e-12, "{brier}");
    Ok(())
}

#[test]
fn no_log_reports_every_point_in_shadow() -> TestResult {
    let home = tempfile::tempdir()?;

    let ran = calibrate(home.path(), &[]);

    assert_eq!(ran.code.as_ref().ok(), Some(&0), "{:?}", ran.code);
    let verdicts: Vec<&Value> = ran.envelope["data"]["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|point| &point["verdict"])
        .collect();
    assert_eq!(verdicts, [&json!("stay-shadow"); 7]);
    Ok(())
}

#[test]
fn a_log_with_no_mod_labels_keeps_the_summary_without_a_via_mod_count() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;

    let ran = calibrate(home.path(), &["--point", "D1"]);

    assert!(
        ran.stderr
            .lines()
            .any(|line| line.starts_with("  noul: n 5, labelled 5, Brier ")),
        "{}",
        ran.stderr
    );
    assert!(!ran.stderr.contains("via mod"), "{}", ran.stderr);
    let noul = &point(&ran.envelope, "D1")["questions"][0];
    assert_eq!(noul["labelled_via_mod"], json!(0));
    Ok(())
}

#[test]
fn mod_labels_are_counted_apart_in_the_envelope_and_the_summary() -> TestResult {
    let home = tempfile::tempdir()?;
    hand_log(home.path())?;
    let relabel = json!({
        "kind": "label", "id": "d1-1-1", "outcome": "yes", "ts_ms": 3,
        "actor": "human", "via": "mod",
    });
    write_log(home.path(), "decisions.1.jsonl", &[relabel])?;

    let ran = calibrate(home.path(), &["--point", "D1"]);

    let noul = &point(&ran.envelope, "D1")["questions"][0];
    assert_eq!(
        (&noul["labelled"], &noul["labelled_via_mod"]),
        (&json!(5), &json!(0)),
        "decisions.1 is older than the live file, so the live label wins"
    );
    write_log(
        home.path(),
        "decisions.jsonl",
        &[
            decision("d1-1-0", "D1", 0.1, "defer"),
            label("d1-1-0", "no"),
            decision("d1-1-1", "D1", 0.3, "defer"),
            label("d1-1-1", "no"),
            json!({"kind": "label", "id": "d1-1-1", "outcome": "yes", "ts_ms": 3, "via": "mod"}),
        ],
    )?;

    let ran = calibrate(home.path(), &["--point", "D1"]);

    let noul = &point(&ran.envelope, "D1")["questions"][0];
    assert_eq!(
        (&noul["labelled"], &noul["labelled_via_mod"]),
        (&json!(2), &json!(1))
    );
    assert!(
        ran.stderr
            .lines()
            .any(|line| line.starts_with("  noul: n 2, labelled 2, via mod 1, Brier ")),
        "{}",
        ran.stderr
    );
    Ok(())
}

mod apply;
