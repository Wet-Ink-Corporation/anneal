//! `anneal orchestrate park` through the built binary, against weigh-in three ways: absent
//! (every platform), a fake `weighin` shell script that records its stdin (unix), and the
//! pinned weigh-in release named by `WEIGHIN_BIN` (CI; the test says it skipped when unset).

use std::error::Error;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

use crate::support::{anneal_in, git, with_fake_clef};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A park of the call "should the cache be shared", blocking ticket 7.
const PARK: [&str; 13] = [
    "orchestrate",
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
    "--resolve-by=2026-10-19",
];

/// The variables through which weigh-in finds a session or a store; removed so the caller's
/// session cannot leak in.
const WEIGH_IN_SESSION: [&str; 4] = [
    "WEIGHIN_SESSION",
    "CLAUDE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ID",
    "WEIGHIN_STORE",
];

/// `anneal <args>` in `dir` with `WEIGHIN_BIN` set to `bin`.
fn anneal(dir: &Path, bin: &Path, args: &[&str]) -> Command {
    let mut command = anneal_in(dir);
    command.args(args).env("WEIGHIN_BIN", bin);
    for var in WEIGH_IN_SESSION {
        command.env_remove(var);
    }
    command
}

/// The envelope on stdout, after checking the exit code is `code`.
fn envelope(output: &Output, code: i32) -> TestResult<Value> {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// The events of type `kind` in the ledger the commands in `dir` use.
fn events(dir: &Path, kind: &str) -> TestResult<Vec<Value>> {
    let output = anneal_in(dir)
        .env("ANNEAL_STORE", dir.join("ledger.sqlite3"))
        .args(["ledger", "query", "--type", kind])
        .output()?;
    let queried = envelope(&output, 0)?;
    // The caller keeps its own copy of each event.
    Ok(queried
        .pointer("/data/events")
        .and_then(Value::as_array)
        .ok_or("events")?
        .iter()
        .filter_map(|recorded| recorded.get("event").cloned())
        .collect())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_weigh_in_absent_the_park_exits_4_and_records_bridge_unavailable() -> TestResult {
    let dir = tempfile::tempdir()?;
    let missing = dir.path().join("no-weighin-here");

    let output = anneal(dir.path(), &missing, &PARK)
        .env("ANNEAL_STORE", dir.path().join("ledger.sqlite3"))
        .output()?;

    let parked = envelope(&output, 4)?;
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(parked["data"]["parked"], json!(false));
    let recorded = events(dir.path(), "bridge.unavailable")?;
    let [event] = recorded.as_slice() else {
        return Err(format!("one bridge.unavailable: {recorded:?}").into());
    };
    assert_eq!(event["ticket"], json!("7"));
    let reason = event["reason"].as_str().unwrap_or_default();
    assert!(
        reason.starts_with("WEIGHIN_BIN names no file: "),
        "{reason}"
    );
    assert_eq!(events(dir.path(), "weighin_parked")?, Vec::<Value>::new());
    Ok(())
}

/// The fake `weighin`: appends its stdin and a newline to `$FAKE_WEIGHIN_LOG`, then answers as
/// weigh-in 0.6.1 does, `already raised` once `$FAKE_WEIGHIN_STATE` exists.
#[cfg(unix)]
const FAKE_WEIGHIN: &str = r#"#!/bin/sh
[ "$1" = batch ] || { echo "fake weighin: only batch" >&2; exit 2; }
cat >> "$FAKE_WEIGHIN_LOG"
echo >> "$FAKE_WEIGHIN_LOG"
if [ -e "$FAKE_WEIGHIN_STATE" ]; then
  echo '✓ wi-e41e02 already raised'
else
  : > "$FAKE_WEIGHIN_STATE"
  echo '✓ raised wi-e41e02 (raised)'
fi
echo '✓ deferred wi-e41e02 until next sitting (default per-tenant by 2026-10-19) · tracking atom .kb/_intake/open_questions/wi-e41e02-x.md'
"#;

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_park_pipes_one_batch_and_a_retry_names_the_same_item_recorded_once() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir()?;
    let bin = dir.path().join("weighin");
    std::fs::write(&bin, FAKE_WEIGHIN)?;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
    let log = dir.path().join("stdin.log");
    let park = || {
        anneal(dir.path(), &bin, &PARK)
            .env("ANNEAL_STORE", dir.path().join("ledger.sqlite3"))
            .env("FAKE_WEIGHIN_LOG", &log)
            .env("FAKE_WEIGHIN_STATE", dir.path().join("raised"))
            .output()
    };

    let first = envelope(&park()?, 0)?;
    let retry = envelope(&park()?, 0)?;

    assert_eq!(first["data"]["item_id"], json!("wi-e41e02"));
    assert_eq!(retry["data"]["item_id"], json!("wi-e41e02"));
    assert_eq!(
        (&first["data"]["recorded"], &retry["data"]["recorded"]),
        (&json!(true), &json!(false))
    );
    let batches: Vec<Value> = std::fs::read_to_string(&log)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(batches.len(), 2, "one batch per park");
    assert_eq!(batches[0], batches[1], "the same batch both times");
    assert_eq!(batches[0][0]["op"], json!("raise"));
    assert_eq!(batches[0][1]["op"], json!("defer"));
    assert_eq!(batches[0][1]["id"], json!("@1"));
    let recorded = events(dir.path(), "weighin_parked")?;
    let parked: Vec<(&Value, &Value, Option<&Value>)> = recorded
        .iter()
        .map(|event| (&event["item_id"], &event["ticket"], event.get("decision")))
        .collect();
    assert_eq!(
        parked,
        [(&json!("wi-e41e02"), &json!("7"), None)],
        "one weighin_parked, with no decision"
    );
    Ok(())
}

/// `weighin <args>` from `bin` in `dir`, with no session; panics unless it exits 0.
fn weighin(bin: &Path, dir: &Path, args: &[&str]) -> TestResult<Vec<u8>> {
    let mut command = Command::new(bin);
    command.args(args).current_dir(dir);
    for var in WEIGH_IN_SESSION {
        command.env_remove(var);
    }
    let output = command.output()?;
    assert!(output.status.success(), "weighin {args:?}: {output:?}");
    Ok(output.stdout)
}

/// Runs the ruled-before hook in `repo` on `prompt` with Clef enforcing D6 and answering
/// `item` at 0.9, returning its stdout parsed.
fn ruled_before(repo: &Path, bin: &Path, item: &str, prompt: &str) -> TestResult<Value> {
    let home = repo.join(".anneal-home");
    let decisions = home.join("decisions");
    std::fs::create_dir_all(&decisions)?;
    std::fs::write(
        decisions.join("counts.json"),
        json!({"kind": "anneal-decision-counts", "version": 1, "answered": {"D6": 50}}).to_string(),
    )?;
    let inner = json!({"probabilities": {item: 0.9, "none": 0.1}});
    let body = json!({"result": {"response": inner.to_string()}}).to_string();
    let input = json!({
        "session_id": "s37",
        "transcript_path": repo.join("session.jsonl"),
        "cwd": repo,
        "hook_event_name": "UserPromptSubmit",
        "prompt": prompt,
    });
    let (output, _served) = with_fake_clef(&body, |url| -> TestResult<Output> {
        let mut child = anneal(repo, bin, &["hook", "ruled-before"])
            .env("ANNEAL_HOME", &home)
            .env("ANNEAL_CLEF_URL", url)
            .env("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123")
            .env("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", "s3cr3t-T0KEN")
            .env("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "enforce")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or("stdin is piped")?
            .write_all(input.to_string().as_bytes())?;
        Ok(child.wait_with_output()?)
    });
    let output = output?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// The exit criterion of Release 5 against the real weigh-in: a park creates one item with
/// its default (a retry adds none), and a ruling recorded with `weighin decide` is surfaced by
/// D6 on a later prompt. Runs only with `WEIGHIN_BIN` naming a weigh-in 0.6 or later.
#[test]
#[cfg_attr(miri, ignore = "runs the binary and weighin")]
fn the_pinned_weigh_in_parks_once_and_its_ruling_reaches_d6() -> TestResult {
    let Some(bin) = std::env::var_os("WEIGHIN_BIN").filter(|bin| !bin.is_empty()) else {
        writeln!(
            std::io::stderr().lock(),
            "skipped: WEIGHIN_BIN is unset; CI sets it to the pinned weigh-in release"
        )?;
        return Ok(());
    };
    let bin = std::path::PathBuf::from(bin);
    let repo = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"]);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );

    let first = envelope(&anneal(repo.path(), &bin, &PARK).output()?, 0)?;
    let retry = envelope(&anneal(repo.path(), &bin, &PARK).output()?, 0)?;

    let item = first["data"]["item_id"].as_str().ok_or("an item id")?;
    assert_eq!(
        retry["data"]["item_id"],
        json!(item),
        "the retry names it again"
    );
    let listed: Value =
        serde_json::from_slice(&weighin(&bin, repo.path(), &["open", "--all", "--json"])?)?;
    let items = listed.as_array().ok_or("a list")?;
    assert_eq!(items.len(), 1, "one item after two parks: {listed}");
    assert_eq!(items[0]["id"], json!(item));
    assert_eq!(items[0]["door_hint"], json!("one-way"));
    assert_eq!(items[0]["deferral"]["default_option"], json!("per-tenant"));
    let open: Value =
        serde_json::from_slice(&weighin(&bin, repo.path(), &["open", "--repo", "--json"])?)?;
    assert_eq!(open[0]["id"], json!(item), "visible in weighin open");

    weighin(&bin, repo.path(), &["decide", item, "shared"])?;
    let surfaced = ruled_before(repo.path(), &bin, item, "let us split the cache per tenant")?;

    let context = surfaced
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        context.contains(item) && context.contains("shared"),
        "D6 surfaces the ruling: {surfaced}"
    );
    Ok(())
}
