//! `anneal calibrate` through `cli`'s argument parsing, with an anneal home of its own.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use serde_json::{Value, json};

use super::{CalibrateArgs, CalibrateEnv, run};
use crate::cli::{Cli, Command};
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
    let env = CalibrateEnv {
        cwd: home,
        // The lookup hands out an owned value per call, as `std::env::var_os` does.
        lookup: |var: &str| -> Option<OsString> {
            (var == "ANNEAL_HOME").then(|| home.as_os_str().to_owned())
        },
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(&args(words), &env, &mut out, &mut err).map(Code::value);
    let envelope = if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).expect("the envelope is JSON")
    };
    Ran {
        code,
        envelope,
        stderr: String::from_utf8(err).expect("the summary is UTF-8"),
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

fn write_log(home: &Path, file: &str, rows: &[Value]) -> std::io::Result<PathBuf> {
    let dir = home.join("decisions");
    fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    let text: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    fs::write(&path, text)?;
    Ok(path)
}

/// The hand set as D1 decisions, labelled.
fn hand_log(home: &Path) -> std::io::Result<()> {
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
fn perfect_d7(home: &Path, n: u32) -> std::io::Result<()> {
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
    fs::write(&thresholds, crate::decide::thresholds::BUILT_IN)?;
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
