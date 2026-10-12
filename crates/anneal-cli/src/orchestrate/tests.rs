//! `anneal orchestrate` through `cli`'s argument parsing, a ledger of its own and a fake
//! `weighin` behind the [`Runner`] port.

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anneal_ledger::{Filter, LedgerEvent, Store};
use clap::Parser;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::{OrchestrateArgs, Ports, run};
use crate::cli::{Cli, Command};
use crate::error::CliError;
use crate::exit_codes::Code;
use crate::weighin::{Call, Ran, RunError, Runner};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// weigh-in 0.6.1's receipt for a first park.
const RECEIPT: &str = "✓ raised wi-e41e02 (raised)\n\
    ✓ deferred wi-e41e02 until next sitting (default per-tenant by 2026-10-19) · tracking atom \
    .kb/_intake/open_questions/wi-e41e02-should-the-cache-be-shared.md\n";

/// A fake `weighin`: one fixed answer, and the stdin of each call.
struct Fake {
    answer: Ran,
    inputs: RefCell<Vec<String>>,
}

impl Fake {
    fn answering(code: i32, stdout: &str) -> Self {
        Self {
            answer: Ran {
                code: Some(code),
                stdout: stdout.as_bytes().to_vec(),
                stderr: b"weighin: something failed\n".to_vec(),
            },
            inputs: RefCell::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.inputs.borrow().len()
    }
}

impl Runner for Fake {
    fn run(&self, call: &Call<'_>) -> Result<Ran, RunError> {
        self.inputs
            .borrow_mut()
            .push(String::from_utf8_lossy(call.input).into_owned());
        // Each call hands out its own copy of the fixed answer.
        Ok(self.answer.clone())
    }
}

/// A working directory with a ledger file and a `weighin` file to point `WEIGHIN_BIN` at.
struct World {
    dir: TempDir,
}

impl World {
    fn new() -> TestResult<Self> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("weighin"), "")?;
        Ok(Self { dir })
    }

    fn cwd(&self) -> &Path {
        self.dir.path()
    }

    fn store(&self) -> PathBuf {
        self.dir.path().join("ledger.sqlite3")
    }

    /// Runs `anneal orchestrate <words>` with `WEIGHIN_BIN` set to `bin` (none: unset, and no
    /// `PATH`), returning the exit code and the envelope.
    fn run(
        &self,
        words: &[&str],
        bin: Option<&Path>,
        runner: &impl Runner,
    ) -> (Result<u8, CliError>, Value) {
        let store = self.store();
        let lookup = |name: &str| -> Option<OsString> {
            match name {
                "ANNEAL_STORE" => Some(store.clone().into_os_string()),
                "WEIGHIN_BIN" => bin.map(|bin| bin.as_os_str().to_owned()),
                _ => None,
            }
        };
        let mut out = Vec::new();
        let ports = Ports {
            cwd: self.cwd(),
            lookup,
            runner,
        };
        let code = run(&args(words), &ports, &mut out).map(Code::value);
        let envelope = if out.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&out).expect("one JSON envelope")
        };
        (code, envelope)
    }

    fn bin(&self) -> PathBuf {
        self.dir.path().join("weighin")
    }

    /// The events in the ledger, as JSON; none without a ledger file.
    fn events(&self) -> TestResult<Vec<LedgerEvent>> {
        if !self.store().exists() {
            return Ok(Vec::new());
        }
        let store = Store::open(&self.store())?;
        Ok(store
            .query(&Filter::default())?
            .into_iter()
            .map(|recorded| recorded.event.event)
            .collect())
    }
}

fn args(words: &[&str]) -> OrchestrateArgs {
    let parsed = Cli::try_parse_from(["anneal", "orchestrate"].iter().chain(words))
        .expect("the arguments parse");
    match parsed.command {
        Some(Command::Orchestrate(args)) => args,
        other => panic!("not orchestrate: {other:?}"),
    }
}

const PARK: [&str; 14] = [
    "park",
    "--ticket",
    "7",
    "--q",
    "Should the cache be shared across tenants?",
    "--default",
    "per-tenant",
    "--flip",
    "a tenant needs isolation",
    "--at",
    "src/cache.rs:12",
    "--resolve-by",
    "2026-10-19",
    "--did=blocked",
];

#[test]
fn a_park_records_the_minted_item_against_the_ticket() -> TestResult {
    let world = World::new()?;
    let fake = Fake::answering(0, RECEIPT);

    let (code, envelope) = world.run(&PARK, Some(&world.bin()), &fake);

    assert_eq!(code.ok(), Some(0), "{envelope}");
    assert_eq!(envelope["data"]["parked"], json!(true));
    assert_eq!(envelope["data"]["item_id"], json!("wi-e41e02"));
    assert_eq!(envelope["data"]["recorded"], json!(true));
    let batch: Value = serde_json::from_str(&fake.inputs.borrow()[0])?;
    assert_eq!(
        batch[0]["blocks"],
        json!("7"),
        "the ticket is what it blocks"
    );
    assert_eq!(batch[1]["default"], json!("per-tenant"));
    assert_eq!(
        world.events()?,
        [LedgerEvent::WeighinParked(
            anneal_ledger::event::WeighinParked {
                item_id: "wi-e41e02".to_owned(),
                ticket: "7".to_owned(),
                decision: None,
            }
        )]
    );
    Ok(())
}

#[test]
fn a_park_run_again_names_the_same_item_and_records_it_once() -> TestResult {
    let world = World::new()?;
    let first = Fake::answering(0, RECEIPT);
    let again = Fake::answering(
        0,
        &RECEIPT.replace("raised wi-e41e02 (raised)", "wi-e41e02 already raised"),
    );

    let (first_code, _) = world.run(&PARK, Some(&world.bin()), &first);
    let (code, envelope) = world.run(&PARK, Some(&world.bin()), &again);

    assert_eq!((first_code.ok(), code.ok()), (Some(0), Some(0)));
    assert_eq!(envelope["data"]["item_id"], json!("wi-e41e02"));
    assert_eq!(
        envelope["data"]["recorded"],
        json!(false),
        "already in the ledger"
    );
    assert_eq!(world.events()?.len(), 1, "one weighin_parked");
    Ok(())
}

#[test]
fn with_weigh_in_absent_the_park_fails_open_and_records_why() -> TestResult {
    let world = World::new()?;
    let fake = Fake::answering(0, RECEIPT);

    let (code, envelope) = world.run(&PARK, None, &fake);

    assert_eq!(code.ok(), Some(4), "{envelope}");
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["data"]["parked"], json!(false));
    assert_eq!(
        envelope["findings"][0]["key"],
        json!("orchestrate/bridge-unavailable:7")
    );
    assert_eq!(fake.calls(), 0, "nothing to run");
    assert_eq!(
        world.events()?,
        [LedgerEvent::BridgeUnavailable(
            anneal_ledger::event::BridgeUnavailable {
                ticket: "7".to_owned(),
                reason: "no weighin: WEIGHIN_BIN is unset and none is on PATH".to_owned(),
            }
        )]
    );
    Ok(())
}

#[test]
fn a_failing_weighin_or_an_unreadable_receipt_fails_open_with_the_reason() -> TestResult {
    let world = World::new()?;
    let failing = Fake::answering(1, RECEIPT);
    let garbled = Fake::answering(0, "✓ ok\n");

    let (failing_code, _) = world.run(&PARK, Some(&world.bin()), &failing);
    let (garbled_code, _) = world.run(&PARK, Some(&world.bin()), &garbled);

    assert_eq!((failing_code.ok(), garbled_code.ok()), (Some(4), Some(4)));
    let reasons: Vec<String> = world
        .events()?
        .into_iter()
        .filter_map(|event| match event {
            LedgerEvent::BridgeUnavailable(unavailable) => Some(unavailable.reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        [
            "weighin exited with 1: weighin: something failed",
            "the receipt names no raised wi- item"
        ]
    );
    Ok(())
}

#[test]
fn an_impossible_date_or_a_line_separator_is_a_usage_error_exiting_2_that_runs_nothing()
-> TestResult {
    let world = World::new()?;
    let fake = Fake::answering(0, RECEIPT);

    for (index, value) in [(12, "2026-02-31"), (4, "Shared\u{2028}cache?")] {
        let mut words = PARK.to_vec();
        words[index] = value;
        let (code, envelope) = world.run(&words, Some(&world.bin()), &fake);
        let error = code.expect_err("refused");
        assert!(
            matches!(error, CliError::Usage { .. }),
            "{value:?}: {error:?}"
        );
        assert_eq!(error.code().value(), 2, "{value:?}");
        assert_eq!(envelope, Value::Null, "nothing on stdout");
    }
    assert_eq!(fake.calls(), 0, "weighin never ran");
    assert!(!world.store().exists(), "no ledger created");
    Ok(())
}

#[test]
fn invalid_park_arguments_are_usage_errors_that_run_and_record_nothing() -> TestResult {
    let world = World::new()?;
    let fake = Fake::answering(0, RECEIPT);
    let cases: [(usize, &str); 5] = [
        (2, "../7"),
        (4, "  "),
        (4, "two\nlines"),
        (12, "next week"),
        (10, ""),
    ];

    for (index, value) in cases {
        let mut words = PARK.to_vec();
        words[index] = value;
        let (code, envelope) = world.run(&words, Some(&world.bin()), &fake);
        assert!(
            matches!(code, Err(CliError::Usage { .. })),
            "{value:?}: {code:?}"
        );
        assert_eq!(envelope, Value::Null, "nothing on stdout");
    }
    assert_eq!(fake.calls(), 0, "weighin never ran");
    assert!(!world.store().exists(), "no ledger created");
    Ok(())
}

#[test]
fn a_ruling_records_what_why_and_cost_if_wrong() -> TestResult {
    let world = World::new()?;
    let fake = Fake::answering(0, RECEIPT);

    let (code, envelope) = world.run(
        &[
            "rule",
            "--ticket",
            "7",
            "--what",
            "keep one cache per tenant",
            "--why",
            "reversible in one commit",
            "--cost",
            "a revert and a cache flush",
        ],
        Some(&world.bin()),
        &fake,
    );

    assert_eq!(code.ok(), Some(0), "{envelope}");
    assert_eq!(envelope["data"]["ticket"], json!("7"));
    assert_eq!(fake.calls(), 0, "a ruling never asks weigh-in");
    assert_eq!(
        world.events()?,
        [LedgerEvent::OrchestrateRuled(
            anneal_ledger::event::OrchestrateRuled {
                ticket: "7".to_owned(),
                what: "keep one cache per tenant".to_owned(),
                why: "reversible in one commit".to_owned(),
                cost_if_wrong: "a revert and a cache flush".to_owned(),
                decision: None,
            }
        )]
    );
    Ok(())
}

/// A markdown tickets directory under `root` holding `tickets` as `(id, blocked_by)`.
fn tickets(root: &Path, tickets: &[(&str, &str)]) -> TestResult {
    let dir = root.join("tickets");
    fs::create_dir_all(&dir)?;
    for (id, blocked_by) in tickets {
        fs::write(
            dir.join(format!("{id}.md")),
            format!("---\ntitle: Ticket {id}\nblocked_by: [{blocked_by}]\n---\nWork.\n"),
        )?;
    }
    Ok(())
}

#[test]
fn fan_out_goes_ahead_with_three_independent_tickets() -> TestResult {
    let world = World::new()?;
    tickets(
        world.cwd(),
        &[
            ("001-a", ""),
            ("002-b", ""),
            ("003-c", ""),
            ("004-d", "001-a"),
        ],
    )?;

    let (code, envelope) = world.run(&["fan-out"], None, &Fake::answering(0, ""));

    assert_eq!(code.ok(), Some(0), "{envelope}");
    assert_eq!(envelope["data"]["fan_out"], json!(true));
    assert_eq!(envelope["data"]["independent"], json!(3));
    assert_eq!(
        envelope["data"]["tickets"],
        json!(["001-a", "002-b", "003-c"])
    );
    Ok(())
}

#[test]
fn fan_out_is_refused_with_fewer_than_three_independent_tickets() -> TestResult {
    let world = World::new()?;
    tickets(
        world.cwd(),
        &[("001-a", ""), ("002-b", ""), ("003-c", "002-b")],
    )?;

    let (code, envelope) = world.run(&["fan-out"], None, &Fake::answering(0, ""));

    assert_eq!(code.ok(), Some(3), "{envelope}");
    assert_eq!(envelope["data"]["fan_out"], json!(false));
    assert_eq!(envelope["data"]["independent"], json!(2));
    assert_eq!(
        envelope["findings"][0]["key"],
        json!("orchestrate/no-fan-out")
    );
    assert!(!world.store().exists(), "fan-out writes nothing");
    Ok(())
}

#[test]
fn fan_out_is_refused_when_the_graph_fails_its_check() -> TestResult {
    let world = World::new()?;
    tickets(
        world.cwd(),
        &[
            ("001-a", "002-b"),
            ("002-b", "001-a"),
            ("003-c", ""),
            ("004-d", ""),
            ("005-e", ""),
        ],
    )?;

    let (code, envelope) = world.run(&["fan-out"], None, &Fake::answering(0, ""));

    assert_eq!(code.ok(), Some(3), "{envelope}");
    assert_eq!(
        envelope["data"]["independent"],
        json!(0),
        "a failed check lists none"
    );
    let keys: Vec<&str> = envelope["findings"]
        .as_array()
        .map(|findings| findings.iter().filter_map(|f| f["key"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(keys, ["dag/cycle:001-a->002-b", "orchestrate/no-fan-out"]);
    Ok(())
}
