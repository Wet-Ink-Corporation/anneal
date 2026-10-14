use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, SystemTimeError};

use serde_json::{Value, json};

use super::{Prepared, Status, status};
use crate::decide::client::{self, Ask, Outcome, Recorded};
use crate::decide::clock::Clock;
use crate::decide::id::OutcomeLabel;
use crate::decide::point::{Point, Question};
use crate::decide::state::CompactState;
use crate::decide::transport::{ClefRequest, Transport, TransportError};

struct Endpoint(Value);

impl Transport for Endpoint {
    fn send(&self, _request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        Ok(json!({"result": {"response": self.0.to_string()}})
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

fn lookup(home: &Path, vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let mut map: BTreeMap<String, OsString> = vars
        .iter()
        .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
        .collect();
    map.insert("ANNEAL_HOME".to_owned(), home.as_os_str().to_owned());
    move |name| map.get(name).cloned()
}

const CONFIGURED: [(&str, &str); 2] = [
    ("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123"),
    ("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", "s3cr3t-T0KEN"),
];

#[test]
fn without_an_account_clef_is_off_and_the_home_is_untouched() {
    let home = tempfile::tempdir().expect("tempdir");

    let status = status(Point::D3, lookup(home.path(), &[]));

    assert!(matches!(status, Status::Off), "{status:?}");
    assert_eq!(
        std::fs::read_dir(home.path()).expect("readable").count(),
        0,
        "nothing created"
    );
}

#[test]
fn a_point_whose_mode_is_off_is_off_and_the_others_on() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut vars = CONFIGURED.to_vec();
    vars.push(("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "shadow,D6=off"));

    let d6 = status(Point::D6, lookup(home.path(), &vars));
    let d3 = status(Point::D3, lookup(home.path(), &vars));

    assert!(matches!(d6, Status::Off), "{d6:?}");
    assert!(matches!(d3, Status::On), "{d3:?}");
}

#[test]
fn a_malformed_mode_is_misconfigured() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut vars = CONFIGURED.to_vec();
    vars.push(("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "loud"));

    let status = status(Point::D3, lookup(home.path(), &vars));

    assert!(matches!(status, Status::Misconfigured(_)), "{status:?}");
}

#[test]
fn a_prepared_setup_asks_logs_and_labels_like_the_command() {
    let home = tempfile::tempdir().expect("tempdir");
    let prepared = Prepared::load(lookup(home.path(), &CONFIGURED)).expect("prepared");
    let options = ["build".to_owned(), "plan".to_owned()];
    let state = CompactState::parse(r#"{"input":"build it"}"#).expect("a state");
    let question = Recorded {
        ask: Ask {
            point: Point::D3,
            question: Question::Choice,
            options: &options,
            state: &state,
        },
        input: None,
    };
    let endpoint = Endpoint(json!({"probabilities": {"build": 0.8, "plan": 0.2}}));

    let asked =
        client::ask_recording(&question, &prepared.setup(), &endpoint, &Fixed).expect("asked");
    let Outcome::Decided(decision) = asked.outcome else {
        panic!("decided: {:?}", asked.outcome);
    };
    let label: OutcomeLabel = "build".parse().expect("a label");
    prepared
        .label(&decision.stamp.id, &label, &Fixed)
        .expect("labelled");

    assert!(asked.unrecorded.is_none(), "{:?}", asked.unrecorded);
    assert_eq!(decision.action, "hint");
    assert_eq!(prepared.log_dir(), home.path().join("decisions"));
    let log = std::fs::read_to_string(home.path().join("decisions").join("decisions.jsonl"))
        .expect("the log");
    let rows: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON row"))
        .collect();
    assert_eq!(rows.len(), 2, "{log}");
    assert_eq!(rows[0]["kind"], json!("decision"));
    assert_eq!(rows[0]["top"], json!("build"));
    assert_eq!(
        rows[1],
        json!({"kind": "label", "id": decision.stamp.id.as_str(), "outcome": "build",
               "ts_ms": 1_760_000_000_000_u64})
    );
}
