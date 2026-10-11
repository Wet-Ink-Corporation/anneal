use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::fs;
use std::time::{Duration, SystemTimeError};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{Ask, AskError, OffReason, Outcome, Setup, TIMEOUT, ask};
use crate::decide::clock::Clock;
use crate::decide::config::{ACCOUNT_VAR, ClefConfig, MODE_VAR, TOKEN_VAR};
use crate::decide::log::DecisionLog;
use crate::decide::point::{Point, Question};
use crate::decide::state::CompactState;
use crate::decide::thresholds::Thresholds;
use crate::decide::transport::{ClefRequest, Transport, TransportError};

const SECRET: &str = "s3cr3t-T0KEN-value";
const ACCOUNT: &str = "acct0123";

/// What the fake endpoint saw of one request.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    url: String,
    model: String,
    bearer: String,
    timeout: Duration,
    body: Value,
}

/// A mocked Workers AI endpoint: records each request and gives one fixed reply.
struct FakeEndpoint {
    reply: Result<Value, TransportError>,
    seen: RefCell<Vec<Seen>>,
}

impl FakeEndpoint {
    /// Answers with `inner` as the model's text, wrapped as Workers AI does.
    fn answering(inner: &Value) -> Self {
        Self {
            reply: Ok(json!({"result": {"response": inner.to_string()}, "success": true})),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn failing(error: TransportError) -> Self {
        Self {
            reply: Err(error),
            seen: RefCell::new(Vec::new()),
        }
    }

    /// The one request seen.
    fn only_request(&self) -> Seen {
        let seen = self.seen.borrow();
        assert_eq!(seen.len(), 1, "one request: {seen:?}");
        seen[0].clone()
    }

    fn requests(&self) -> usize {
        self.seen.borrow().len()
    }
}

impl Transport for FakeEndpoint {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        self.seen.borrow_mut().push(Seen {
            url: request.url.clone(),
            model: request.model.to_owned(),
            bearer: request.token.expose_secret().to_owned(),
            timeout: request.timeout,
            // A body that is not JSON shows as null, which the body test would catch.
            body: serde_json::from_slice(&request.body).unwrap_or(Value::Null),
        });
        // Each call hands out its own copy of the fixed reply.
        self.reply
            .clone()
            .map(|value| value.to_string().into_bytes())
    }
}

/// Fixed wall time; the monotonic clock advances 39 ms per reading.
struct FakeClock {
    readings: Cell<u64>,
}

impl Clock for FakeClock {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        Ok(1_760_000_000_000)
    }

    fn monotonic(&self) -> Duration {
        let reading = self.readings.get();
        self.readings.set(reading + 1);
        Duration::from_millis(reading * 39)
    }
}

/// An anneal home, the shipped thresholds and a configuration from `vars`.
struct World {
    _home: TempDir,
    thresholds: Thresholds,
    config: ClefConfig,
    log: DecisionLog,
    state: CompactState,
}

impl World {
    fn with(vars: &[(&str, &str)]) -> Self {
        let home = tempfile::tempdir().expect("tempdir");
        let log = DecisionLog::in_home(home.path());
        let config = ClefConfig::from_env(|name| {
            vars.iter()
                .find(|(var, _)| *var == name)
                .map(|(_, value)| OsString::from(value))
        })
        .expect("a valid configuration");
        Self {
            _home: home,
            thresholds: Thresholds::built_in().expect("shipped thresholds"),
            config,
            log,
            state: CompactState::parse(r#"{"tool":"Bash","input":{"command":"rm -r x"}}"#)
                .expect("a state"),
        }
    }

    fn configured() -> Self {
        Self::with(&[(ACCOUNT_VAR, ACCOUNT), (TOKEN_VAR, SECRET)])
    }

    /// The question `point` asks in these tests, with `options`.
    fn question<'a>(&'a self, point: Point, options: &'a [String]) -> Ask<'a> {
        Ask {
            point,
            question: question_of(point),
            options,
            state: &self.state,
        }
    }

    fn ask(&self, request: &Ask<'_>, endpoint: &FakeEndpoint) -> Result<Outcome<'_>, AskError> {
        let setup = Setup {
            thresholds: &self.thresholds,
            config: &self.config,
            log: &self.log,
        };
        let clock = FakeClock {
            readings: Cell::new(0),
        };
        ask(request, &setup, endpoint, &clock)
    }

    /// Asks D1's noul question.
    fn ask_d1(&self, endpoint: &FakeEndpoint) -> Outcome<'_> {
        self.ask(&self.question(Point::D1, &[]), endpoint)
            .expect("D1 asks noul")
    }

    fn rows(&self) -> Vec<Value> {
        fs::read_to_string(self.log.path())
            .map(|text| {
                text.lines()
                    .map(|line| serde_json::from_str(line).expect("a JSON row"))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The question each point is asked in these tests.
fn question_of(point: Point) -> Question {
    match point {
        Point::D3 | Point::D6 => Question::Choice,
        Point::D5 => Question::Score,
        Point::D1 | Point::D2 | Point::D4 | Point::D7 => Question::Noul,
    }
}

fn phases() -> Vec<String> {
    vec!["plan".to_owned(), "build".to_owned()]
}

fn yes(p: f64) -> Value {
    json!({"probabilities": {"yes": p}, "confidence": 0.5})
}

#[test]
fn each_point_calls_its_model_with_the_token_and_the_2_s_timeout() {
    let world = World::configured();
    for point in Point::ALL {
        let endpoint = FakeEndpoint::answering(&json!({
            "probabilities": {"yes": 0.5, "plan": 0.5, "build": 0.5},
            "score": 0.5
        }));
        let options = if question_of(point) == Question::Choice {
            phases()
        } else {
            Vec::new()
        };

        let outcome = world.ask(&world.question(point, &options), &endpoint);

        assert!(
            matches!(outcome, Ok(Outcome::Decided(_))),
            "{point}: {outcome:?}"
        );
        let model = if point == Point::D4 {
            "@cf/wet-ink/clef"
        } else {
            "@cf/wet-ink/clef-flash"
        };
        let seen = endpoint.only_request();
        assert_eq!(
            seen.url,
            format!("https://api.cloudflare.com/client/v4/accounts/{ACCOUNT}/ai/run/{model}"),
            "{point}"
        );
        assert_eq!(seen.model, model, "{point}");
        assert_eq!(seen.bearer, SECRET, "{point}");
        assert_eq!(seen.timeout, TIMEOUT, "{point}");
    }
}

#[test]
fn the_request_puts_the_question_first_and_the_compact_state_last() {
    let world = World::configured();
    let endpoint = FakeEndpoint::answering(&yes(0.5));

    world.ask_d1(&endpoint);

    assert_eq!(
        endpoint.only_request().body,
        json!({"messages": [
            {"role": "system", "content": json!({
                "point": "D1",
                "question": "noul",
                "subject": "destructive_or_irreversible",
                "options": ["yes", "no"],
            }).to_string()},
            {"role": "user", "content": world.state.as_str()},
        ]})
    );
}

#[test]
fn the_threshold_applies_to_the_probability_not_the_confidence() {
    let world = World::configured();
    let cases = [
        (
            json!({"probabilities": {"yes": 0.95}, "confidence": 0.1}),
            "deny",
        ),
        (
            json!({"probabilities": {"yes": 0.5}, "confidence": 0.99}),
            "ask",
        ),
        (
            json!({"probabilities": {"yes": 0.1}, "confidence": 0.99}),
            "defer",
        ),
    ];
    for (inner, expected) in cases {
        let outcome = world.ask_d1(&FakeEndpoint::answering(&inner));

        let Outcome::Decided(decision) = outcome else {
            panic!("decided: {outcome:?}");
        };
        assert_eq!(decision.action, expected, "{inner}");
    }
}

#[test]
fn a_decision_appends_one_row_with_the_documented_fields() {
    let world = World::configured();
    let endpoint = FakeEndpoint::answering(&json!({"probabilities": {"yes": 0.95, "no": 0.05}}));

    let outcome = world.ask_d1(&endpoint);

    let Outcome::Decided(decision) = outcome else {
        panic!("decided: {outcome:?}");
    };
    assert_eq!(
        world.rows(),
        [json!({
            "kind": "decision",
            "id": decision.stamp.id.as_str(),
            "ts_ms": 1_760_000_000_000_u64,
            "point": "D1",
            "question": "noul",
            "subject": "destructive_or_irreversible",
            "model": "@cf/wet-ink/clef-flash",
            "state_sha256": world.state.sha256(),
            "probabilities": {"yes": 0.95, "no": 0.05},
            "top": null,
            "probability": 0.95,
            "thresholds": {
                "subject": "destructive_or_irreversible",
                "bands": [
                    {"at_least": 0.9, "action": "deny"},
                    {"at_least": 0.4, "action": "ask"},
                ],
                "otherwise": "defer",
            },
            "threshold": 0.9,
            "action": "deny",
            "mode": "shadow",
            "configured_mode": "shadow",
            "latency_ms": 39,
            "outcome": null,
            "unreachable": false,
            "error": null,
            "error_reason": null,
        })]
    );
}

#[test]
fn unreachable_or_timed_out_is_no_answer_with_the_row_marked_unreachable() {
    let failures = [
        (
            TransportError::Unreachable {
                reason: "connection refused".to_owned(),
            },
            "unreachable",
        ),
        (TransportError::TimedOut { after: TIMEOUT }, "timed_out"),
    ];
    for (failure, kind) in failures {
        let world = World::configured();

        let outcome = world.ask_d1(&FakeEndpoint::failing(failure));

        let Outcome::NoAnswer(no_answer) = outcome else {
            panic!("no answer: {outcome:?}");
        };
        assert_eq!(no_answer.fallback, "ask", "D1 falls back to ask");
        let rows = world.rows();
        assert_eq!(rows.len(), 1, "one row");
        assert_eq!(rows[0]["unreachable"], json!(true), "{kind}");
        assert_eq!(rows[0]["error"], json!(kind));
        assert_eq!(rows[0]["action"], Value::Null);
        assert_eq!(rows[0]["probabilities"], Value::Null);
    }
}

#[test]
fn a_response_without_the_probability_is_a_bad_response_not_unreachable() {
    let world = World::configured();
    let endpoint = FakeEndpoint::answering(&json!({"confidence": 0.9}));

    let outcome = world.ask(&world.question(Point::D7, &[]), &endpoint);

    let Ok(Outcome::NoAnswer(no_answer)) = outcome else {
        panic!("no answer: {outcome:?}");
    };
    assert_eq!(no_answer.fallback, "fail_open");
    let rows = world.rows();
    assert_eq!(rows.len(), 1, "one row");
    assert_eq!(rows[0]["unreachable"], json!(false));
    assert_eq!(rows[0]["error"], json!("bad_response"));
}

#[test]
fn without_a_token_clef_is_unreachable_and_not_called() {
    let world = World::with(&[(ACCOUNT_VAR, ACCOUNT)]);
    let endpoint = FakeEndpoint::answering(&yes(0.9));

    let outcome = world.ask_d1(&endpoint);

    assert!(matches!(outcome, Outcome::NoAnswer(_)), "{outcome:?}");
    assert_eq!(endpoint.requests(), 0, "no request without a token");
    assert_eq!(world.rows()[0]["unreachable"], json!(true));
}

#[test]
fn off_by_mode_or_without_an_account_asks_nothing_and_logs_nothing() {
    let cases = [
        (
            World::with(&[
                (ACCOUNT_VAR, ACCOUNT),
                (TOKEN_VAR, SECRET),
                (MODE_VAR, "shadow,D1=off"),
            ]),
            OffReason::ModeOff,
        ),
        (World::with(&[(TOKEN_VAR, SECRET)]), OffReason::NoAccount),
    ];
    for (world, reason) in cases {
        let endpoint = FakeEndpoint::answering(&yes(0.9));

        let outcome = world.ask_d1(&endpoint);

        assert!(
            matches!(outcome, Outcome::Off(off) if off == reason),
            "{outcome:?}"
        );
        assert_eq!(endpoint.requests(), 0, "Clef is not called");
        assert!(!world.log.path().exists(), "nothing logged");
    }
}

#[test]
fn enforce_stays_shadow_for_the_first_50_answered_decisions() {
    let world = World::with(&[
        (ACCOUNT_VAR, ACCOUNT),
        (TOKEN_VAR, SECRET),
        (MODE_VAR, "enforce"),
    ]);
    for _ in 0..49 {
        world.log.count_answered(Point::D1).expect("counted");
    }

    let fiftieth = world.ask_d1(&FakeEndpoint::answering(&yes(0.9)));
    let fifty_first = world.ask_d1(&FakeEndpoint::answering(&yes(0.9)));

    let modes: Vec<(Value, Value)> = world
        .rows()
        .into_iter()
        .map(|mut row| (row["mode"].take(), row["configured_mode"].take()))
        .collect();
    assert_eq!(
        modes,
        [
            (json!("shadow"), json!("enforce")),
            (json!("enforce"), json!("enforce")),
        ]
    );
    assert!(matches!(fiftieth, Outcome::Decided(_)));
    assert!(matches!(fifty_first, Outcome::Decided(_)));
}

#[test]
fn an_unanswered_call_does_not_count_toward_the_50() {
    let world = World::configured();

    world.ask_d1(&FakeEndpoint::failing(TransportError::TimedOut {
        after: TIMEOUT,
    }));

    assert_eq!(world.log.answered(Point::D1).expect("counts"), 0);
}

#[test]
fn a_question_the_point_does_not_ask_is_refused_before_anything_runs() {
    let world = World::configured();
    let endpoint = FakeEndpoint::answering(&yes(0.9));
    let options = phases();
    let mut request = world.question(Point::D1, &options);
    request.question = Question::Choice;

    let result = world.ask(&request, &endpoint);

    assert!(matches!(
        result,
        Err(AskError::Unsupported {
            point: Point::D1,
            question: Question::Choice
        })
    ));
    assert_eq!(endpoint.requests(), 0);
}

#[test]
fn options_are_required_for_a_choice_and_refused_otherwise() {
    let world = World::configured();
    let endpoint = FakeEndpoint::answering(&yes(0.9));
    let one = ["only".to_owned()];
    let duplicated = ["a".to_owned(), "a".to_owned()];
    let too_long = ["a".to_owned(), "b".repeat(65)];
    let phases = phases();

    for (point, options) in [
        (Point::D3, &[][..]),
        (Point::D3, &one[..]),
        (Point::D3, &duplicated[..]),
        (Point::D3, &too_long[..]),
        (Point::D1, &phases[..]),
    ] {
        let result = world.ask(&world.question(point, options), &endpoint);
        assert!(
            matches!(result, Err(AskError::Options { .. })),
            "{point} {options:?}: {result:?}"
        );
    }
}

#[test]
fn the_token_never_appears_in_rows_outcomes_or_errors() {
    let world = World::configured();
    let endpoints = [
        FakeEndpoint::answering(&yes(0.9)),
        FakeEndpoint::answering(&json!({"nothing": true})),
        FakeEndpoint::failing(TransportError::Unreachable {
            reason: "dns".to_owned(),
        }),
        FakeEndpoint::failing(TransportError::TimedOut { after: TIMEOUT }),
    ];
    for endpoint in &endpoints {
        let outcome = world.ask_d1(endpoint);
        assert!(!format!("{outcome:?}").contains(SECRET), "{outcome:?}");
    }
    let options = phases();
    let mut refused = world.question(Point::D1, &options);
    refused.question = Question::Choice;
    let error = world.ask(&refused, &endpoints[0]);

    let log = fs::read_to_string(world.log.path()).expect("the log");
    assert!(!log.contains(SECRET), "log rows");
    assert!(!format!("{error:?}").contains(SECRET), "errors");
    assert!(!format!("{:?}", world.config).contains(SECRET), "config");
    assert_eq!(world.rows().len(), endpoints.len(), "one row per call");
}
