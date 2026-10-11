//! `anneal decide` through `cli`'s argument parsing and envelope, with a fake endpoint.

use std::ffi::OsString;
use std::time::{Duration, SystemTimeError};

use clap::Parser;
use serde_json::{Value, json};

use super::{DecideArgs, DecideEnv, run};
use crate::cli::{Cli, Command};
use crate::decide::clock::Clock;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::error::CliError;
use crate::exit_codes::Code;

const SECRET: &str = "s3cr3t-T0KEN-value";

/// Answers every request with one fixed reply.
struct Endpoint(Result<Value, TransportError>);

impl Transport for Endpoint {
    fn send(&self, _request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        // Each call hands out its own copy of the fixed reply.
        self.0.clone().map(|inner| {
            json!({"result": {"response": inner.to_string()}})
                .to_string()
                .into_bytes()
        })
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

fn args(words: &[&str]) -> DecideArgs {
    let parsed =
        Cli::try_parse_from(["anneal", "decide"].iter().chain(words)).expect("the arguments parse");
    match parsed.command {
        Some(Command::Decide(args)) => args,
        other => panic!("not decide: {other:?}"),
    }
}

/// Runs `anneal decide <words>` with `vars` and an anneal home of its own, returning the exit
/// code, the envelope and the log's text.
fn decide(
    words: &[&str],
    vars: &[(&str, &str)],
    endpoint: &Endpoint,
) -> (Result<u8, CliError>, Value, String) {
    let home = tempfile::tempdir().expect("tempdir");
    let home_path = home.path().to_owned();
    let lookup = |name: &str| {
        if name == "ANNEAL_HOME" {
            return Some(home_path.clone().into_os_string());
        }
        vars.iter()
            .find(|(var, _)| *var == name)
            .map(|(_, value)| OsString::from(value))
    };
    let env = DecideEnv {
        cwd: home.path(),
        lookup,
        transport: endpoint,
        clock: &Fixed,
    };
    let mut out = Vec::new();
    let code = run(&args(words), &env, &mut out).map(Code::value);
    let envelope = if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).expect("one JSON envelope")
    };
    let log = std::fs::read_to_string(home.path().join("decisions").join("decisions.jsonl"))
        .unwrap_or_default();
    (code, envelope, log)
}

const CONFIGURED: [(&str, &str); 2] = [
    ("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123"),
    ("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", SECRET),
];

const D1: [&str; 6] = [
    "--point",
    "D1",
    "--question",
    "noul",
    "--state",
    r#"{"tool":"Bash","input":{"command":"rm -r build"}}"#,
];

#[test]
fn a_decision_exits_0_with_its_action_and_mode() {
    let endpoint = Endpoint(Ok(json!({"probabilities": {"yes": 0.95}})));

    let (code, envelope, log) = decide(&D1, &CONFIGURED, &endpoint);

    assert_eq!(code.ok(), Some(0));
    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(envelope["command"], json!("decide"));
    let data = &envelope["data"];
    assert_eq!(data["point"], json!("D1"));
    assert_eq!(data["action"], json!("deny"));
    assert_eq!(data["threshold"], json!(0.9));
    assert_eq!(data["mode"], json!("shadow"));
    assert_eq!(data["enforce"], json!(false), "shadow: callers do not act");
    assert_eq!(data["model"], json!("@cf/wet-ink/clef-flash"));
    assert_eq!(log.lines().count(), 1, "one row");
}

#[test]
fn no_answer_exits_5_with_the_fallback_and_one_finding() {
    let endpoint = Endpoint(Err(TransportError::TimedOut {
        after: Duration::from_secs(2),
    }));

    let (code, envelope, log) = decide(&D1, &CONFIGURED, &endpoint);

    assert_eq!(code.ok(), Some(5));
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["data"]["fallback"], json!("ask"));
    assert_eq!(envelope["data"]["error"], json!("timed_out"));
    assert_eq!(envelope["data"]["unreachable"], json!(true));
    assert_eq!(envelope["findings"][0]["key"], json!("decide/no-answer:D1"));
    assert!(log.contains(r#""unreachable":true"#), "{log}");
}

#[test]
fn clef_off_exits_3_and_logs_nothing() {
    let endpoint = Endpoint(Ok(json!({"probabilities": {"yes": 0.95}})));

    let (code, envelope, log) = decide(&D1, &[], &endpoint);

    assert_eq!(code.ok(), Some(3));
    assert_eq!(envelope["data"]["reason"], json!("no_account"));
    assert_eq!(log, "", "nothing logged");
}

#[test]
fn the_off_envelope_reports_the_configured_mode_beside_the_reason() {
    let endpoint = Endpoint(Ok(json!({"probabilities": {"yes": 0.95}})));
    let mode_off = [
        CONFIGURED[0],
        CONFIGURED[1],
        ("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "enforce,D1=off"),
    ];
    let no_account = [("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "enforce")];

    for (vars, reason, configured) in [
        (&mode_off[..], "mode_off", "off"),
        (&no_account[..], "no_account", "enforce"),
    ] {
        let (code, envelope, _) = decide(&D1, vars, &endpoint);

        assert_eq!(code.ok(), Some(3), "{reason}");
        assert_eq!(envelope["data"]["reason"], json!(reason));
        assert_eq!(
            envelope["data"]["configured_mode"],
            json!(configured),
            "{reason}"
        );
    }
}

#[test]
fn a_malformed_mode_token_or_account_exits_5_with_the_fallback_and_the_reason() {
    let endpoint = Endpoint(Ok(json!({"probabilities": {"yes": 0.95}})));
    let bad_token = "bad token\u{7}value";
    let cases = [
        (
            ("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "loud"),
            "CLAUDE_PLUGIN_OPTION_CLEF_MODE",
        ),
        (
            ("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", bad_token),
            "CLAUDE_PLUGIN_OPTION_CF_API_TOKEN",
        ),
        (
            ("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct/../x"),
            "CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID",
        ),
    ];
    for (bad, var) in cases {
        let vars: Vec<(&str, &str)> = CONFIGURED
            .into_iter()
            .filter(|(name, _)| *name != bad.0)
            .chain([bad])
            .collect();

        let (code, envelope, log) = decide(&D1, &vars, &endpoint);

        assert_eq!(code.ok(), Some(5), "{var}: {envelope}");
        let data = &envelope["data"];
        assert_eq!(data["fallback"], json!("ask"), "{var}");
        assert_eq!(data["error"], json!("misconfigured"), "{var}");
        assert_eq!(data["unreachable"], json!(false), "{var}");
        let reason = data["reason"].as_str().unwrap_or_default();
        assert!(reason.contains(var), "{var}: {reason}");
        assert_eq!(envelope["findings"][0]["key"], json!("decide/no-answer:D1"));
        assert!(!envelope.to_string().contains("bad token"), "{envelope}");
        assert_eq!(log, "", "{var}: nothing is asked, so nothing is logged");
    }
}

#[test]
fn a_usage_error_still_wins_over_a_malformed_configuration() {
    let endpoint = Endpoint(Ok(json!({})));
    let words = ["--point", "D1", "--question", "choice", "--state", "{}"];

    let (code, _, _) = decide(
        &words,
        &[("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "loud")],
        &endpoint,
    );

    assert_eq!(code.map_err(|error| error.code().value()), Err(2));
}

#[test]
fn a_state_that_is_not_an_object_is_a_usage_error() {
    let endpoint = Endpoint(Ok(json!({})));
    let words = ["--point", "D1", "--question", "noul", "--state", "[1]"];

    let (code, envelope, _) = decide(&words, &CONFIGURED, &endpoint);

    assert_eq!(code.map_err(|error| error.code().value()), Err(2));
    assert_eq!(envelope, Value::Null, "nothing on stdout");
}

#[test]
fn label_appends_a_label_row_for_the_id() {
    let endpoint = Endpoint(Ok(json!({})));

    let (code, envelope, log) = decide(
        &["label", "d1760000000000-42-0", "destructive"],
        &[],
        &endpoint,
    );

    assert_eq!(code.ok(), Some(0));
    assert_eq!(envelope["command"], json!("decide label"));
    let row: Value = serde_json::from_str(log.trim_end()).expect("one JSON row");
    assert_eq!(
        row,
        json!({
            "kind": "label",
            "id": "d1760000000000-42-0",
            "outcome": "destructive",
            "ts_ms": 1_760_000_000_000_u64,
        })
    );
}

#[test]
fn a_malformed_label_id_is_a_usage_error_and_appends_nothing() {
    let (code, _, log) = decide(&["label", "not-an-id", "x"], &[], &Endpoint(Ok(json!({}))));

    assert_eq!(code.map_err(|error| error.code().value()), Err(2));
    assert_eq!(log, "");
}

#[test]
fn the_token_is_in_no_envelope_or_row() {
    let replies = [
        Ok(json!({"probabilities": {"yes": 0.5}})),
        Ok(json!({})),
        Err(TransportError::Unreachable {
            reason: "refused".to_owned(),
        }),
    ];
    for reply in replies {
        let (_, envelope, log) = decide(&D1, &CONFIGURED, &Endpoint(reply));

        assert!(!envelope.to_string().contains(SECRET), "{envelope}");
        assert!(!log.contains(SECRET), "{log}");
    }
}
