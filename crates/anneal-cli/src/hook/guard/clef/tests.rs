//! D1 through the guard's own entry point, with a fake Workers AI endpoint and fixed clocks.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTimeError};

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{EventKind, Filter, LedgerEvent, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::Ports;
use crate::decide::clock::Clock;
use crate::decide::config::{ACCOUNT_VAR, MODE_VAR, TOKEN_VAR};
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::hook::HookEnv;
use crate::hook::guard::run_with;

const SECRET: &str = "s3cr3t-T0KEN-value";

/// A mocked Workers AI endpoint: records each request body and gives one fixed reply.
struct FakeEndpoint {
    reply: Result<Value, TransportError>,
    bodies: RefCell<Vec<Value>>,
}

impl FakeEndpoint {
    /// Answers `yes` with probability `p`, wrapped as Workers AI does.
    fn yes(p: f64) -> Self {
        let inner = json!({"probabilities": {"yes": p, "no": 1.0 - p}});
        Self {
            reply: Ok(json!({"result": {"response": inner.to_string()}, "success": true})),
            bodies: RefCell::new(Vec::new()),
        }
    }

    fn failing(error: TransportError) -> Self {
        Self {
            reply: Err(error),
            bodies: RefCell::new(Vec::new()),
        }
    }

    fn unreachable() -> Self {
        Self::failing(TransportError::Unreachable {
            reason: "no HTTP client configured (pending dependency decision)".to_owned(),
        })
    }

    fn bad_response() -> Self {
        Self {
            reply: Ok(json!({"success": false})),
            bodies: RefCell::new(Vec::new()),
        }
    }

    fn requests(&self) -> usize {
        self.bodies.borrow().len()
    }
}

impl Transport for FakeEndpoint {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        self.bodies
            .borrow_mut()
            .push(serde_json::from_slice(&request.body).unwrap_or(Value::Null));
        // Each call hands out its own copy of the fixed reply.
        self.reply
            .clone()
            .map(|value| value.to_string().into_bytes())
    }
}

/// Fixed wall time and a monotonic clock that stands still.
struct FixedClock;

impl Clock for FixedClock {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        Ok(1_760_000_000_000)
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// Fixed wall time and a monotonic clock that moves `step` on every reading, so a call to
/// Clef (two readings) looks slow.
struct SteppingClock {
    step: Duration,
    readings: Cell<u32>,
}

impl Clock for SteppingClock {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        Ok(1_760_000_000_000)
    }

    fn monotonic(&self) -> Duration {
        let reading = self.readings.get();
        self.readings.set(reading + 1);
        self.step * reading
    }
}

/// An anneal home, a ledger path and the environment the guard sees.
struct World {
    dir: TempDir,
    vars: BTreeMap<&'static str, OsString>,
}

impl World {
    /// Clef off: no account.
    fn off() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut vars = BTreeMap::new();
        vars.insert("ANNEAL_HOME", dir.path().join("home").into_os_string());
        vars.insert(
            "ANNEAL_STORE",
            dir.path().join("ledger.sqlite3").into_os_string(),
        );
        Self { dir, vars }
    }

    /// An account and a token, in `mode`.
    fn configured(mode: &str) -> Self {
        let mut world = Self::off();
        world.set(ACCOUNT_VAR, "acct0123");
        world.set(TOKEN_VAR, SECRET);
        world.set(MODE_VAR, mode);
        world
    }

    /// Enforce mode past the shadow period (50 answered D1 decisions counted).
    fn enforcing() -> Self {
        let world = Self::configured("enforce");
        let decisions = world.home().join("decisions");
        fs::create_dir_all(&decisions).expect("the log directory");
        fs::write(
            decisions.join("counts.json"),
            json!({"kind": "anneal-decision-counts", "version": 1, "answered": {"D1": 50}})
                .to_string(),
        )
        .expect("counts written");
        world
    }

    fn set(&mut self, var: &'static str, value: &str) {
        self.vars.insert(var, OsString::from(value));
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// Runs the guard on a `tool_name` call with `command`, and returns its stdout.
    fn run_tool(&self, tool: &str, command: &str, endpoint: &FakeEndpoint) -> String {
        let ports = Ports {
            transport: endpoint,
            clock: &FixedClock,
        };
        self.run_ports(tool, command, &ports)
    }

    /// Runs the guard on a Bash call with `command`, reading time from `clock`.
    fn run_timed(&self, command: &str, endpoint: &FakeEndpoint, clock: &impl Clock) -> String {
        let ports = Ports {
            transport: endpoint,
            clock,
        };
        self.run_ports("Bash", command, &ports)
    }

    fn run_ports(
        &self,
        tool: &str,
        command: &str,
        ports: &Ports<'_, FakeEndpoint, impl Clock>,
    ) -> String {
        let stdin = json!({
            "session_id": "s-d1",
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": {"command": command},
        })
        .to_string();
        let env = HookEnv {
            stdin: stdin.as_bytes(),
            current_dir: || Ok(PathBuf::from("/")),
            lookup: |var: &str| self.vars.get(var).cloned(),
        };
        let mut out = Vec::new();
        run_with(env, ports, &mut out).expect("a Vec accepts every write");
        String::from_utf8(out).expect("UTF-8")
    }

    fn guard(&self, command: &str, endpoint: &FakeEndpoint) -> String {
        self.run_tool("Bash", command, endpoint)
    }

    /// The decision rows in the log.
    fn rows(&self) -> Vec<Value> {
        fs::read_to_string(self.home().join("decisions").join("decisions.jsonl"))
            .map(|text| {
                text.lines()
                    .map(|line| serde_json::from_str(line).expect("a JSON row"))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The `point`s of the `gate_blocked` events in the ledger.
    fn blocked(&self) -> Vec<String> {
        let store = self.dir.path().join("ledger.sqlite3");
        if !store.exists() {
            return Vec::new();
        }
        Store::open(&store)
            .and_then(|ledger| {
                ledger.query(&Filter {
                    types: vec![EventKind::GateBlocked],
                    ..Filter::default()
                })
            })
            .expect("the ledger reads")
            .into_iter()
            .map(|stored| match stored.event.event {
                LedgerEvent::GateBlocked(GateBlocked { point, .. }) => point,
                other => panic!("a gate_blocked event: {other:?}"),
            })
            .collect()
    }
}

/// The decision and reason of a `PreToolUse` permission decision on stdout.
fn permission(stdout: &str) -> (String, String) {
    let output: Value = serde_json::from_str(stdout).expect("one JSON object");
    let field = |name: &str| {
        output
            .pointer(&format!("/hookSpecificOutput/{name}"))
            .and_then(Value::as_str)
            .expect("the field")
            .to_owned()
    };
    let (decision, reason) = (
        field("permissionDecision"),
        field("permissionDecisionReason"),
    );
    assert_eq!(
        output,
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }}),
        "a PreToolUse decision and nothing else"
    );
    assert_eq!(stdout.lines().count(), 1, "one line");
    (decision, reason)
}

/// Every answer and failure Clef can give.
fn every_reply() -> [FakeEndpoint; 5] {
    [
        FakeEndpoint::yes(0.0),
        FakeEndpoint::yes(0.5),
        FakeEndpoint::yes(0.95),
        FakeEndpoint::unreachable(),
        FakeEndpoint::bad_response(),
    ]
}

#[test]
#[cfg_attr(miri, ignore = "opens a SQLite ledger")]
fn shadow_mode_never_changes_the_guards_verdict() {
    let commands = ["rm -r build", "git push -f origin main", "ls"];
    let off = World::off();
    let shadow = World::configured("shadow");
    let enforce_in_its_shadow_period = World::configured("enforce");

    for command in commands {
        let expected = off.guard(command, &FakeEndpoint::yes(0.0));
        for endpoint in every_reply() {
            for world in [&shadow, &enforce_in_its_shadow_period] {
                assert_eq!(
                    world.guard(command, &endpoint),
                    expected,
                    "{command:?} with {:?}",
                    endpoint.reply
                );
            }
        }
    }
    let modes: Vec<Value> = shadow
        .rows()
        .iter()
        .map(|row| row["mode"].clone())
        .collect();
    assert_eq!(
        modes,
        vec![json!("shadow"); 10],
        "allowed commands are asked and logged"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens a SQLite ledger")]
fn enforced_d1_denies_at_0_90_and_records_gate_blocked() {
    let world = World::enforcing();

    let stdout = world.guard("rm -r build", &FakeEndpoint::yes(0.95));

    let (decision, reason) = permission(&stdout);
    assert_eq!(decision, "deny");
    assert!(
        reason.starts_with("ERROR guard destructive: Clef D1 "),
        "{reason}"
    );
    assert_eq!(world.blocked(), ["guard.destructive"]);
}

#[test]
fn enforced_d1_asks_from_0_40() {
    let world = World::enforcing();

    let stdout = world.guard("rm -r build", &FakeEndpoint::yes(0.5));

    let (decision, reason) = permission(&stdout);
    assert_eq!(decision, "ask");
    assert!(reason.contains("Clef D1"), "{reason}");
    assert!(world.blocked().is_empty(), "an ask is not a block");
}

#[test]
fn enforced_d1_defers_below_0_40() {
    let world = World::enforcing();

    let stdout = world.guard("rm -r build", &FakeEndpoint::yes(0.1));

    assert_eq!(stdout, "", "defer: the normal permission flow decides");
}

#[test]
fn enforced_d1_asks_when_clef_gives_no_answer() {
    let world = World::enforcing();

    for endpoint in [FakeEndpoint::unreachable(), FakeEndpoint::bad_response()] {
        let stdout = world.guard("rm -r build", &endpoint);

        let (decision, reason) = permission(&stdout);
        assert_eq!(decision, "ask", "{:?}", endpoint.reply);
        assert!(reason.contains("could not judge"), "{reason}");
    }
    let unreachable: Vec<Value> = world
        .rows()
        .iter()
        .map(|row| row["unreachable"].clone())
        .collect();
    assert_eq!(unreachable, [json!(true), json!(false)], "each call logged");
}

#[test]
fn a_fallback_other_than_ask_never_denies() {
    let mut world = World::enforcing();
    let mut thresholds: Value =
        serde_json::from_str(crate::decide::thresholds::BUILT_IN).expect("shipped JSON");
    thresholds["points"]["D1"]["fallback"] = json!("deny");
    let playbooks = world.dir.path().join("plugin").join("playbooks");
    fs::create_dir_all(&playbooks).expect("the plugin's playbooks");
    fs::write(
        playbooks.join("clef-thresholds.json"),
        thresholds.to_string(),
    )
    .expect("thresholds written");
    let root = world.dir.path().join("plugin");
    world
        .vars
        .insert("CLAUDE_PLUGIN_ROOT", root.into_os_string());

    let stdout = world.guard("rm -r build", &FakeEndpoint::unreachable());

    assert_eq!(stdout, "", "a Clef failure is never a deny");
    assert_eq!(world.rows().len(), 1, "the call is logged");
}

#[test]
fn with_clef_off_nothing_is_asked_or_logged() {
    let mut off_for_d1 = World::configured("enforce");
    off_for_d1.set(MODE_VAR, "enforce,D1=off");
    for world in [World::off(), off_for_d1] {
        let endpoint = FakeEndpoint::yes(0.95);

        let stdout = world.guard("rm -r build", &endpoint);

        assert_eq!(stdout, "", "allowed, as before D1");
        assert_eq!(endpoint.requests(), 0, "no call");
        assert!(world.rows().is_empty(), "no row");
    }
}

#[test]
fn a_malformed_configuration_leaves_the_verdict_alone() {
    let world = World::configured("loud");
    let endpoint = FakeEndpoint::yes(0.95);

    let stdout = world.guard("rm -r build", &endpoint);

    assert_eq!(stdout, "", "a Clef problem never blocks");
    assert_eq!(endpoint.requests(), 0, "no call");
}

#[test]
#[cfg_attr(miri, ignore = "opens a SQLite ledger")]
fn a_denylist_denial_never_reaches_clef() {
    let world = World::enforcing();
    let endpoint = FakeEndpoint::yes(0.0);

    let stdout = world.guard("git push -f origin main", &endpoint);

    let (decision, reason) = permission(&stdout);
    assert_eq!(decision, "deny");
    assert!(reason.starts_with("ERROR guard force-push: "), "{reason}");
    assert_eq!(endpoint.requests(), 0, "no call");
    assert!(world.rows().is_empty(), "no row");
}

#[test]
fn another_tool_never_reaches_clef() {
    let world = World::enforcing();
    let endpoint = FakeEndpoint::yes(0.95);

    let stdout = world.run_tool("Write", "rm -r build", &endpoint);

    assert_eq!(stdout, "");
    assert_eq!(endpoint.requests(), 0, "no call");
}

#[test]
fn the_row_and_the_state_hold_the_trimmed_command_without_secrets() {
    let world = World::configured("shadow");
    let endpoint = FakeEndpoint::yes(0.5);
    let command = "  \n AWS_SECRET_ACCESS_KEY=hunter2 export GH_TOKEN=ghp_x; deploy --token=tok-123 --password pw-456 \n";

    world.guard(command, &endpoint);

    let redacted = "AWS_SECRET_ACCESS_KEY=<redacted> export <redacted>; deploy --token=<redacted> --password <redacted>";
    let rows = world.rows();
    let [row] = rows.as_slice() else {
        panic!("one row: {rows:?}");
    };
    assert_eq!(row["input"], json!(redacted), "the row's command");
    let bodies = endpoint.bodies.borrow();
    let state = bodies[0]
        .pointer("/messages/1/content")
        .and_then(Value::as_str)
        .expect("the state message");
    assert_eq!(
        serde_json::from_str::<Value>(state).expect("the state is JSON"),
        json!({"tool": "Bash", "input": {"command": redacted}}),
        "the state sent to Clef"
    );
    let log = fs::read_to_string(world.home().join("decisions").join("decisions.jsonl"))
        .expect("the log");
    for secret in ["hunter2", "ghp_x", "tok-123", "pw-456", SECRET] {
        assert!(!log.contains(secret), "{secret} in the log");
        assert!(!state.contains(secret), "{secret} sent to Clef");
    }
}

#[test]
fn in_enforce_mode_unreadable_counts_ask_rather_than_allow() {
    let world = World::configured("enforce");
    fs::create_dir_all(world.home().join("decisions").join("counts.json"))
        .expect("a directory where counts.json goes");
    let endpoint = FakeEndpoint::yes(0.95);

    let stdout = world.guard("rm -r build", &endpoint);

    let (decision, reason) = permission(&stdout);
    assert_eq!(
        decision, "ask",
        "a D1 failure in enforce mode is never an allow"
    );
    assert!(reason.contains("could not run"), "{reason}");
}

#[test]
fn in_shadow_mode_an_unwritable_log_leaves_the_verdict_alone() {
    let world = World::configured("shadow");
    fs::create_dir_all(world.home().join("decisions").join("decisions.jsonl"))
        .expect("a directory where the log goes");

    let stdout = world.guard("rm -r build", &FakeEndpoint::yes(0.95));

    assert_eq!(stdout, "");
}

#[test]
#[cfg_attr(miri, ignore = "opens a SQLite ledger")]
fn in_enforce_mode_an_answer_that_cannot_be_logged_keeps_its_deny() {
    let world = World::enforcing();
    fs::create_dir_all(world.home().join("decisions").join("decisions.jsonl"))
        .expect("a directory where the log goes");

    let stdout = world.guard("rm -r build", &FakeEndpoint::yes(0.95));

    let (decision, reason) = permission(&stdout);
    assert_eq!(decision, "deny");
    assert!(reason.contains("the decision was not logged"), "{reason}");
}

#[test]
fn in_enforce_mode_a_deferred_answer_that_cannot_be_logged_asks() {
    let world = World::enforcing();
    fs::create_dir_all(world.home().join("decisions").join("decisions.jsonl"))
        .expect("a directory where the log goes");

    let stdout = world.guard("rm -r build", &FakeEndpoint::yes(0.1));

    let (decision, reason) = permission(&stdout);
    assert_eq!(decision, "ask");
    assert!(reason.contains("the decision was not logged"), "{reason}");
}

#[test]
fn a_band_without_a_threshold_names_none() {
    let mut world = World::enforcing();
    let mut thresholds: Value =
        serde_json::from_str(crate::decide::thresholds::BUILT_IN).expect("shipped JSON");
    thresholds["points"]["D1"]["rules"]["noul"]["otherwise"] = json!("ask");
    let playbooks = world.dir.path().join("plugin").join("playbooks");
    fs::create_dir_all(&playbooks).expect("the plugin's playbooks");
    fs::write(
        playbooks.join("clef-thresholds.json"),
        thresholds.to_string(),
    )
    .expect("thresholds written");
    let root = world.dir.path().join("plugin");
    world
        .vars
        .insert("CLAUDE_PLUGIN_ROOT", root.into_os_string());

    let stdout = world.guard("rm -r build", &FakeEndpoint::yes(0.1));

    let (decision, reason) = permission(&stdout);
    assert_eq!(decision, "ask");
    assert!(reason.contains("(p 0.10)"), "{reason}");
    assert!(!reason.contains(" from "), "{reason}");
}

#[test]
fn a_deny_after_a_slow_clef_skips_the_ledger_record_to_stay_in_the_hook_budget() {
    let world = World::enforcing();
    // Start, Clef's start and end, then the deny: 4.5 s spent, nothing left to record with.
    let clock = SteppingClock {
        step: Duration::from_millis(1_500),
        readings: Cell::new(0),
    };

    let stdout = world.run_timed("rm -r build", &FakeEndpoint::yes(0.95), &clock);

    let (decision, reason) = permission(&stdout);
    assert_eq!(decision, "deny");
    assert!(
        reason.ends_with(
            "(the block was not recorded in the ledger: no time left in the hook's budget)"
        ),
        "{reason}"
    );
    assert!(world.blocked().is_empty(), "no ledger record was attempted");
}
