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

/// D2, `/orchestrate`'s question: is this call a one-way door?
const D2: [&str; 6] = [
    "--point",
    "D2",
    "--question",
    "noul",
    "--state",
    r#"{"tool":"orchestrate","input":{"call":"share the cache across tenants"}}"#,
];

#[test]
fn d2_parks_a_one_way_door_from_half_and_rules_below_it_in_shadow_by_default() {
    let at_half = Endpoint(Ok(json!({"probabilities": {"yes": 0.5}})));
    let below = Endpoint(Ok(json!({"probabilities": {"yes": 0.49}})));

    let (half_code, half, _) = decide(&D2, &CONFIGURED, &at_half);
    let (below_code, under, _) = decide(&D2, &CONFIGURED, &below);

    assert_eq!((half_code.ok(), below_code.ok()), (Some(0), Some(0)));
    assert_eq!(half["data"]["action"], json!("park"));
    assert_eq!(half["data"]["threshold"], json!(0.5));
    assert_eq!(under["data"]["action"], json!("rule"));
    assert_eq!(half["data"]["mode"], json!("shadow"), "D2 starts in shadow");
    assert_eq!(
        half["data"]["enforce"],
        json!(false),
        "in shadow the orchestrator rules"
    );
}

#[test]
fn d2_without_an_answer_falls_back_to_failing_open() {
    let unreachable = Endpoint(Err(TransportError::TimedOut {
        after: Duration::from_secs(2),
    }));

    let (code, envelope, _) = decide(&D2, &CONFIGURED, &unreachable);

    assert_eq!(code.ok(), Some(5));
    assert_eq!(envelope["data"]["fallback"], json!("fail_open"));
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
fn label_records_its_actor_and_via_on_the_row_and_in_data() {
    let endpoint = Endpoint(Ok(json!({})));
    let words = [
        "label",
        "d1760000000000-42-0",
        "yes",
        "--actor",
        "human",
        "--via",
        "mod",
    ];

    let (code, envelope, log) = decide(&words, &[], &endpoint);

    assert_eq!(code.ok(), Some(0));
    let row: Value = serde_json::from_str(log.trim_end()).expect("one JSON row");
    assert_eq!(
        row,
        json!({
            "kind": "label",
            "id": "d1760000000000-42-0",
            "outcome": "yes",
            "ts_ms": 1_760_000_000_000_u64,
            "actor": "human",
            "via": "mod",
        })
    );
    assert_eq!(
        (&envelope["data"]["actor"], &envelope["data"]["via"]),
        (&json!("human"), &json!("mod"))
    );
}

#[test]
fn a_malformed_label_id_is_a_usage_error_and_appends_nothing() {
    let (code, _, log) = decide(&["label", "not-an-id", "x"], &[], &Endpoint(Ok(json!({}))));

    assert_eq!(code.map_err(|error| error.code().value()), Err(2));
    assert_eq!(log, "");
}

#[test]
fn label_with_only_an_actor_leaves_via_off_the_row_and_data() {
    let endpoint = Endpoint(Ok(json!({})));
    let words = ["label", "d1760000000000-42-0", "yes", "--actor", "model"];

    let (code, envelope, log) = decide(&words, &[], &endpoint);

    assert_eq!(code.ok(), Some(0));
    let row: Value = serde_json::from_str(log.trim_end()).expect("one JSON row");
    assert_eq!(
        row,
        json!({
            "kind": "label",
            "id": "d1760000000000-42-0",
            "outcome": "yes",
            "ts_ms": 1_760_000_000_000_u64,
            "actor": "model",
        })
    );
    let data = envelope["data"].as_object().expect("data is an object");
    assert_eq!(
        (data.get("actor"), data.contains_key("via")),
        (Some(&json!("model")), false),
        "{envelope}"
    );
}

#[test]
fn label_without_flags_has_no_actor_or_via_in_data() {
    let endpoint = Endpoint(Ok(json!({})));

    let (code, envelope, _) = decide(&["label", "d1760000000000-42-0", "yes"], &[], &endpoint);

    assert_eq!(code.ok(), Some(0));
    let data = envelope["data"].as_object().expect("data is an object");
    assert_eq!(
        (data.contains_key("actor"), data.contains_key("via")),
        (false, false),
        "{envelope}"
    );
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

#[test]
fn a_thresholds_file_that_is_not_utf8_is_its_own_error_before_any_parse()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("clef-thresholds.json");
    std::fs::write(&path, b"{\"version\": 1, \"models\": \"\xff\"}")?;

    let read = super::read_thresholds_text(Some(&path));

    assert!(
        matches!(&read, Err(super::DecideError::ThresholdsNotUtf8 { path: named, .. }) if *named == path),
        "{read:?}"
    );
    assert_eq!(
        read.map(|_read| ()).map_err(|error| error.code()),
        Err(crate::exit_codes::INTERNAL)
    );
    Ok(())
}
