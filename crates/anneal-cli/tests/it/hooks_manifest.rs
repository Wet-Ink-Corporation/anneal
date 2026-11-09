//! `plugin/hooks/hooks.json` and the CLI's hook metadata table (`anneal_cli::hook::metadata`):
//! every hook entry is exec form with a failure policy and a timeout, every entry has exactly
//! one metadata row and every row an entry (same id, event, matcher, fingerprint and failure
//! policy), and every row is well formed. CI runs this through `cargo test`.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anneal_cli::cli::Cli;
use anneal_cli::hook::HookName;
use anneal_cli::hook::metadata::{HookMetadata, Role};
use clap::CommandFactory;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::support::{anneal_in, anneal_with_home};

type TestResult = Result<(), Box<dyn Error>>;

/// What every hook command runs: the launcher, by path, with no shell.
const LAUNCHER: &str = "${CLAUDE_PLUGIN_ROOT}/libexec/anneal";

/// The patterns a metadata row's fields match.
const ID_PATTERN: &str = "^[a-z][a-z0-9-]*$";
const FINGERPRINT_PATTERN: &str = "^sha256:[0-9a-f]{16}$";
const MODEL_GENERATION_PATTERN: &str = "^claude-[0-9]+(\\.[0-9]+)?$";

fn hooks_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/hooks")
}

#[test]
fn the_plugin_hooks_directory_holds_only_the_manifest() -> TestResult {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(hooks_dir())? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    assert_eq!(
        names,
        vec!["hooks.json".to_owned()],
        "hook metadata lives in the binary (hook::metadata); a plugin holds no data files"
    );
    Ok(())
}

fn read_json(name: &str) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_str(&std::fs::read_to_string(
        hooks_dir().join(name),
    )?)?)
}

/// The Claude Code events a hooks.json entry may be declared under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum HookEvent {
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolBatch,
    PermissionRequest,
    Stop,
    SubagentStop,
    SubagentStart,
    PreCompact,
    PostCompact,
    Notification,
    TaskCompleted,
}

/// Claude Code's `onFailure`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OnFailure {
    Continue,
    Block,
}

/// `hooks.json`, strictly.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HooksFile {
    hooks: BTreeMap<HookEvent, Vec<MatcherGroup>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatcherGroup {
    #[serde(default)]
    matcher: Option<String>,
    hooks: Vec<Value>,
}

/// A hook entry: exec form (`command` plus `args`), an explicit timeout and a failure policy.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandHook {
    #[serde(rename = "type")]
    kind: String,
    command: String,
    args: Vec<String>,
    timeout: u32,
    #[serde(rename = "onFailure")]
    on_failure: OnFailure,
}

/// A hooks.json entry, as the metadata describes it.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    event: HookEvent,
    matcher: String,
    fingerprint: String,
    on_failure: OnFailure,
}

/// `value` as compact JSON with every object's keys sorted, so the fingerprint does not depend
/// on key order.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, String> =
                map.iter().map(|(k, v)| (k, canonical(v))).collect();
            let fields: Vec<String> = sorted
                .iter()
                .map(|(k, v)| format!("{}:{v}", Value::from(k.as_str())))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        scalar => scalar.to_string(),
    }
}

/// `sha256:` and the first 16 hex digits of SHA-256 over `<event>\n<matcher>\n<hook JSON>`.
fn fingerprint(event: HookEvent, matcher: &str, hook: &Value) -> Result<String, Box<dyn Error>> {
    let event = serde_json::to_value(event)?;
    let event = event.as_str().ok_or("an event serializes as a string")?;
    let digest = Sha256::digest(format!("{event}\n{matcher}\n{}", canonical(hook)).as_bytes());
    let hex: String = digest
        .iter()
        .take(8)
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect();
    Ok(format!("sha256:{hex}"))
}

/// Every hooks.json entry, checked to be exec form with a timeout and a failure policy.
fn entries() -> Result<Vec<Entry>, Box<dyn Error>> {
    Ok(declared()?.into_iter().map(|(entry, _)| entry).collect())
}

/// A hooks.json entry and the full `args` it passes to the launcher.
type Declared = (Entry, Vec<String>);

/// Every hooks.json entry with the full `args` it passes to the launcher, checked to be exec
/// form with a timeout and a failure policy.
fn declared() -> Result<Vec<Declared>, Box<dyn Error>> {
    let file: HooksFile = serde_json::from_value(read_json("hooks.json")?)?;
    let mut entries = Vec::new();
    for (event, groups) in file.hooks {
        for group in groups {
            let matcher = group.matcher.unwrap_or_default();
            for raw in &group.hooks {
                let hook: CommandHook = serde_json::from_value(raw.clone())?;
                assert_eq!(hook.kind, "command", "{raw}: a command hook");
                assert_eq!(hook.command, LAUNCHER, "{raw}: the launcher, by path");
                assert!(hook.timeout > 0, "{raw}: an explicit, positive timeout");
                let [verb, id, ..] = hook.args.as_slice() else {
                    return Err(format!("{raw}: args are `hook <id> ...`").into());
                };
                assert_eq!(verb, "hook", "{raw}: runs `anneal hook <id>`");
                let entry = Entry {
                    id: id.clone(),
                    event,
                    fingerprint: fingerprint(event, &matcher, raw)?,
                    matcher: matcher.clone(),
                    on_failure: hook.on_failure,
                };
                entries.push((entry, hook.args));
            }
        }
    }
    Ok(entries)
}

/// Every hook with its row in the CLI's metadata table.
fn rows() -> impl Iterator<Item = (HookName, HookMetadata)> {
    HookName::ALL
        .into_iter()
        .map(|hook| (hook, hook.metadata()))
}

/// The hooks.json entry a row describes, its event and policy read back through the spellings
/// hooks.json uses.
fn described(hook: HookName, row: HookMetadata) -> Result<Entry, Box<dyn Error>> {
    Ok(Entry {
        id: hook.as_str().to_owned(),
        event: serde_json::from_value(Value::from(row.event.as_str()))?,
        matcher: row.matcher.to_owned(),
        fingerprint: row.fingerprint.to_owned(),
        on_failure: serde_json::from_value(Value::from(row.on_failure.as_str()))?,
    })
}

/// Runs `anneal <args>` as Claude Code would run a hook entry: `{}` on stdin, in a scratch
/// directory, with the ledger in that directory. Returns the exit code.
fn run_hook_args(args: &[String]) -> Result<Option<i32>, Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let mut child = anneal_in(scratch.path())
        .args(args)
        .env("ANNEAL_STORE", scratch.path().join("ledger.sqlite3"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin is piped")?;
    match stdin.write_all(b"{}") {
        // A usage error exits before reading stdin; the exit code below still reports it.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
        written => written?,
    }
    drop(stdin);
    Ok(child.wait_with_output()?.status.code())
}

#[test]
fn every_hooks_json_entry_is_exec_form_with_a_failure_policy_and_a_timeout() -> TestResult {
    let entries = entries()?;
    assert!(!entries.is_empty(), "hooks.json declares hooks");
    Ok(())
}

/// A hooks.json entry under its event and matcher.
#[derive(Debug)]
struct Placed {
    event: HookEvent,
    matcher: String,
    hook: CommandHook,
}

/// Every hooks.json entry with its event and matcher.
fn command_hooks() -> Result<Vec<Placed>, Box<dyn Error>> {
    let file: HooksFile = serde_json::from_value(read_json("hooks.json")?)?;
    let mut hooks = Vec::new();
    for (event, groups) in file.hooks {
        for group in groups {
            let matcher = group.matcher.unwrap_or_default();
            for raw in group.hooks {
                hooks.push(Placed {
                    event,
                    // Each entry of the group records the group's matcher.
                    matcher: matcher.clone(),
                    hook: serde_json::from_value(raw)?,
                });
            }
        }
    }
    Ok(hooks)
}

#[test]
fn the_guard_hook_runs_before_every_bash_call_and_fails_closed() -> TestResult {
    let guards: Vec<Placed> = command_hooks()?
        .into_iter()
        .filter(|placed| placed.hook.args.get(1).map(String::as_str) == Some("guard"))
        .collect();
    let [
        Placed {
            event,
            matcher,
            hook,
        },
    ] = guards.as_slice()
    else {
        return Err(format!("one guard entry, got {guards:?}").into());
    };
    assert_eq!(*event, HookEvent::PreToolUse, "before the tool runs");
    assert_eq!(matcher, "Bash", "on every Bash call");
    assert_eq!(
        hook.args,
        ["hook", "guard", "--fail-closed"],
        "the launcher blocks when no binary is installed"
    );
    assert_eq!(
        hook.on_failure,
        OnFailure::Block,
        "a crash, a timeout or bad output blocks"
    );
    assert_eq!(hook.timeout, 5, "the guard budget is 5 s");
    assert_eq!(
        HookName::Guard.metadata().role,
        Role::Guard,
        "its metadata row says guard"
    );
    Ok(())
}

#[test]
fn the_preconditions_hook_runs_before_every_edit_and_write_and_fails_closed() -> TestResult {
    let gates: Vec<Placed> = command_hooks()?
        .into_iter()
        .filter(|placed| {
            placed.hook.args.get(1).map(String::as_str) == Some(HookName::Preconditions.as_str())
        })
        .collect();
    let [
        Placed {
            event,
            matcher,
            hook,
        },
    ] = gates.as_slice()
    else {
        return Err(format!("one preconditions entry, got {gates:?}").into());
    };
    assert_eq!(*event, HookEvent::PreToolUse, "before the tool runs");
    assert_eq!(
        matcher, "Edit|Write|MultiEdit|NotebookEdit",
        "on every tool that edits a file (#80)"
    );
    assert_eq!(
        hook.args,
        ["hook", "preconditions", "--fail-closed"],
        "the launcher blocks when no binary is installed"
    );
    assert_eq!(
        hook.on_failure,
        OnFailure::Block,
        "a crash, a timeout or bad output blocks"
    );
    assert_eq!(hook.timeout, 5, "the preconditions budget is 5 s");
    assert_eq!(
        HookName::Preconditions.metadata().role,
        Role::Guard,
        "its metadata row says guard"
    );
    Ok(())
}

/// The one hooks.json entry running `anneal hook <name>`.
fn only_entry(name: HookName) -> Result<Placed, Box<dyn Error>> {
    let mut found: Vec<Placed> = command_hooks()?
        .into_iter()
        .filter(|placed| placed.hook.args.get(1).map(String::as_str) == Some(name.as_str()))
        .collect();
    match (found.pop(), found.is_empty()) {
        (Some(placed), true) => Ok(placed),
        (last, _) => Err(format!("one {} entry, got {found:?} and {last:?}", name.as_str()).into()),
    }
}

#[test]
fn the_injection_scan_hook_runs_once_per_batch_with_a_short_timeout_and_fails_open() -> TestResult {
    let Placed {
        event,
        matcher,
        hook,
    } = only_entry(HookName::InjectionScan)?;
    assert_eq!(
        event,
        HookEvent::PostToolBatch,
        "after each batch's Read and WebFetch results, before the next model request"
    );
    assert_eq!(matcher, "", "PostToolBatch takes no matcher");
    assert_eq!(hook.args, ["hook", "injection-scan"], "no --fail-closed");
    assert_eq!(hook.on_failure, OnFailure::Continue, "never blocks");
    assert_eq!(hook.timeout, 2, "a short timeout");
    Ok(())
}

#[test]
fn the_completeness_hook_runs_on_every_stop_and_fails_open() -> TestResult {
    let Placed {
        event,
        matcher,
        hook,
    } = only_entry(HookName::Completeness)?;
    assert_eq!(event, HookEvent::Stop, "when the main agent stops");
    assert_eq!(matcher, "", "Stop takes no matcher");
    assert_eq!(hook.args, ["hook", "completeness"], "no --fail-closed");
    assert_eq!(hook.on_failure, OnFailure::Continue, "fails open");
    assert_eq!(hook.timeout, 30, "the completeness budget is 30 s");
    Ok(())
}

#[test]
fn the_status_hook_runs_on_every_subagent_stop_and_fails_open() -> TestResult {
    let Placed {
        event,
        matcher,
        hook,
    } = only_entry(HookName::Status)?;
    assert_eq!(event, HookEvent::SubagentStop, "when a subagent stops");
    assert_eq!(matcher, "", "every agent type");
    assert_eq!(hook.args, ["hook", "status"], "no --fail-closed");
    assert_eq!(hook.on_failure, OnFailure::Continue, "fails open");
    assert_eq!(hook.timeout, 15, "the status budget is 15 s");
    Ok(())
}

#[test]
fn the_route_and_ruled_before_hooks_run_on_every_prompt_and_fail_open() -> TestResult {
    for (name, id) in [
        (HookName::Route, "route"),
        (HookName::RuledBefore, "ruled-before"),
    ] {
        let Placed {
            event,
            matcher,
            hook,
        } = only_entry(name)?;
        assert_eq!(event, HookEvent::UserPromptSubmit, "{id}: on every prompt");
        assert_eq!(matcher, "", "{id}: UserPromptSubmit takes no matcher");
        assert_eq!(hook.args, ["hook", id], "{id}: no --fail-closed");
        assert_eq!(hook.on_failure, OnFailure::Continue, "{id}: fails open");
        assert_eq!(
            hook.timeout, 5,
            "{id}: one 2 s Clef call, plus reading and logging"
        );
    }
    Ok(())
}

#[test]
fn the_fast_check_hook_runs_once_per_batch_of_tool_calls_and_fails_open() -> TestResult {
    let placed = command_hooks()?;
    let fast: Vec<&Placed> = placed
        .iter()
        .filter(|placed| {
            placed.hook.args.get(1).map(String::as_str) == Some(HookName::FastCheck.as_str())
        })
        .collect();
    let [
        Placed {
            event,
            matcher,
            hook,
        },
    ] = fast.as_slice()
    else {
        return Err(format!("one fast-check entry, got {fast:?}").into());
    };
    assert_eq!(
        *event,
        HookEvent::PostToolBatch,
        "once per batch of parallel tool calls (PostToolUse would run once per call)"
    );
    assert_eq!(matcher, "", "PostToolBatch takes no matcher");
    assert_eq!(hook.args, ["hook", "fast-check"], "no --fail-closed");
    assert_eq!(hook.on_failure, OnFailure::Continue, "fails open");
    assert_eq!(hook.timeout, 15, "the fast-check budget is 15 s");
    assert!(
        placed
            .iter()
            .all(|placed| placed.event != HookEvent::PostToolUse),
        "no per-call hook runs the checks again"
    );
    Ok(())
}

#[test]
fn an_entry_blocks_on_failure_exactly_when_it_passes_fail_closed() -> TestResult {
    for Placed { hook, .. } in command_hooks()? {
        let fail_closed = hook.args.iter().any(|arg| arg == "--fail-closed");
        assert_eq!(
            hook.on_failure == OnFailure::Block,
            fail_closed,
            "{:?}: onFailure block and --fail-closed go together, so the binary, the launcher \
             and Claude Code agree",
            hook.args
        );
    }
    Ok(())
}

#[test]
fn the_bootstrap_hook_runs_on_every_session_start_and_fails_open() -> TestResult {
    let bootstrap: Vec<Entry> = entries()?
        .into_iter()
        .filter(|entry| entry.id == HookName::Bootstrap.as_str())
        .collect();
    let [entry] = bootstrap.as_slice() else {
        return Err(format!("one bootstrap entry, got {bootstrap:?}").into());
    };
    assert_eq!(entry.event, HookEvent::SessionStart, "on SessionStart");
    assert_eq!(
        entry.matcher, "startup|clear|compact|resume",
        "on every source but fork"
    );
    assert_eq!(entry.on_failure, OnFailure::Continue, "fails open");
    Ok(())
}

#[test]
fn every_metadata_row_is_well_formed() {
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let hex = |s: &str| {
        s.len() == 16
            && s.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    };
    for (hook, row) in rows() {
        let id = hook.as_str();
        let id_ok = id.starts_with(|c: char| c.is_ascii_lowercase())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        assert!(id_ok, "{id}: id matches {ID_PATTERN}");
        assert!(
            row.fingerprint.strip_prefix("sha256:").is_some_and(hex),
            "{id}: fingerprint {} matches {FINGERPRINT_PATTERN}",
            row.fingerprint
        );
        let generation = row.model_generation.strip_prefix("claude-").unwrap_or("");
        let (major, minor) = generation.split_once('.').unwrap_or((generation, "0"));
        assert!(
            digits(major) && digits(minor),
            "{id}: model_generation {} matches {MODEL_GENERATION_PATTERN}",
            row.model_generation
        );
        assert!(!row.source.trim().is_empty(), "{id}: names its source");
    }
}

#[test]
fn every_hooks_json_entry_has_one_metadata_row_and_every_row_an_entry() -> TestResult {
    let mut from_hooks = entries()?;
    from_hooks.sort_by(|a, b| a.id.cmp(&b.id));
    let mut from_rows: Vec<Entry> = rows()
        .map(|(hook, row)| described(hook, row))
        .collect::<Result<_, _>>()?;
    from_rows.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        from_rows, from_hooks,
        "the metadata table (crates/anneal-cli/src/hook/metadata.rs) matches hooks.json entries \
         one to one (id, event, matcher, fingerprint, on_failure); after changing an entry, copy \
         its new fingerprint into its row"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn every_metadata_id_is_an_anneal_hook_the_binary_runs() -> TestResult {
    let ids: BTreeSet<&str> = rows().map(|(hook, _)| hook.as_str()).collect();
    let cli = Cli::command();
    let known: BTreeSet<&str> = cli
        .find_subcommand("hook")
        .ok_or("anneal has a hook command")?
        .get_subcommands()
        .map(clap::Command::get_name)
        .filter(|name| *name != "help")
        .collect();
    assert_eq!(ids, known, "one metadata row per `anneal hook` subcommand");
    let home = tempfile::tempdir()?;
    for id in &ids {
        let output = anneal_with_home(&hooks_dir(), home.path())
            .args(["hook", id, "--help"])
            .output()?;
        assert_eq!(output.status.code(), Some(0), "anneal hook {id} --help");
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn every_hooks_json_entry_runs_with_its_exact_args_and_exits_0() -> TestResult {
    for (entry, args) in declared()? {
        assert_eq!(
            run_hook_args(&args)?,
            Some(0),
            "{}: `anneal {}` exits 0 (a usage error would exit 2, a block)",
            entry.id,
            args.join(" ")
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_entry_with_an_argument_the_binary_rejects_would_not_exit_0() -> TestResult {
    // The control for the test above: running the exact args does catch a bad argument.
    let (entry, mut args) = declared()?
        .into_iter()
        .next()
        .ok_or("hooks.json declares a hook")?;
    args.push("--no-such-flag".to_owned());

    let code = run_hook_args(&args)?;

    assert_eq!(code, Some(2), "{}: a usage error exits 2", entry.id);
    Ok(())
}

#[test]
fn the_context_monitor_runs_once_per_batch_of_tool_calls_and_fails_open() -> TestResult {
    let Placed {
        event,
        matcher,
        hook,
    } = only_entry(HookName::ContextMonitor)?;
    assert_eq!(
        event,
        HookEvent::PostToolBatch,
        "once per batch (no PostToolUse entry may run per call)"
    );
    assert_eq!(matcher, "", "PostToolBatch takes no matcher");
    assert_eq!(hook.args, ["hook", "context-monitor"], "no --fail-closed");
    assert_eq!(hook.on_failure, OnFailure::Continue, "fails open");
    assert_eq!(hook.timeout, 5, "it reads two small files");
    Ok(())
}

#[test]
fn the_handoff_hook_runs_before_every_compaction_and_fails_open() -> TestResult {
    let Placed {
        event,
        matcher,
        hook,
    } = only_entry(HookName::Handoff)?;
    assert_eq!(event, HookEvent::PreCompact, "before compaction");
    assert_eq!(matcher, "", "manual and automatic compactions");
    assert_eq!(hook.args, ["hook", "handoff"], "no --fail-closed");
    assert_eq!(
        hook.on_failure,
        OnFailure::Continue,
        "never blocks a compaction"
    );
    assert_eq!(
        hook.timeout, 30,
        "the Stop hook's checks run inside it, within the same 30 s"
    );
    Ok(())
}

#[test]
fn the_brief_hook_runs_before_every_agent_spawn_and_fails_closed() -> TestResult {
    let Placed {
        event,
        matcher,
        hook,
    } = only_entry(HookName::Brief)?;
    assert_eq!(event, HookEvent::PreToolUse, "before the spawn");
    assert_eq!(matcher, "Agent", "on every Agent call");
    assert_eq!(
        hook.args,
        ["hook", "brief", "--fail-closed"],
        "the launcher blocks when no binary is installed"
    );
    assert_eq!(
        hook.on_failure,
        OnFailure::Block,
        "a crash or timeout blocks"
    );
    assert_eq!(hook.timeout, 5, "the brief budget is 5 s");
    assert_eq!(
        HookName::Brief.metadata().role,
        Role::Guard,
        "its metadata row says guard"
    );
    Ok(())
}

#[test]
fn the_standing_orders_hook_runs_on_every_subagent_start_and_fails_open() -> TestResult {
    let Placed {
        event,
        matcher,
        hook,
    } = only_entry(HookName::StandingOrders)?;
    assert_eq!(event, HookEvent::SubagentStart, "when a subagent starts");
    assert_eq!(matcher, "", "every agent type");
    assert_eq!(hook.args, ["hook", "standing-orders"], "no --fail-closed");
    assert_eq!(hook.on_failure, OnFailure::Continue, "fails open");
    assert_eq!(hook.timeout, 5, "the standing-orders budget is 5 s");
    Ok(())
}
