//! `anneal mode` and `anneal hook preconditions` through the built binary, on scratch git
//! repositories holding the dry-run define chain: each build precondition missing, then
//! satisfied; diagnose before and after a red reproduction; the paths that are always allowed;
//! silence with no mode; and failing closed on bad input.

use std::error::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, copy_dir, git};

type TestResult = Result<(), Box<dyn Error>>;

const PLAN: &str = "define/plan.md";
const SPEC: &str = "define/spec.md";
const TREE: &str = "define/design-tree.json";
const SOURCE: &str = "src/main.rs";
const QUOTE: &str = "Yes, that's it: only notes list, compact JSON, go ahead.";

/// The dry-run fixture: plan, spec and design tree under `define/`, its glossary and
/// constitution at the root, where `lint spec` looks for them.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/fixtures/dry-run")
}

/// A scratch repository with the define chain, a source file and a test file, all committed.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Result<Self, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        git(root, &["init", "-q"]);
        // Byte-for-byte content on every platform, and no signing prompt on CI runners.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        let define = root.join("define");
        copy_dir(&fixture(), &define)?;
        for name in ["CONTEXT.md", "constitution.md"] {
            std::fs::rename(define.join(name), root.join(name))?;
        }
        let repo = Self { dir };
        repo.write(SOURCE, "fn main() {}\n")?;
        repo.write("tests/cli.rs", "#[test]\nfn t() {}\n")?;
        repo.commit("base");
        Ok(repo)
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, content: &str) -> io::Result<()> {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)
    }

    fn commit(&self, message: &str) {
        git(self.path(), &["add", "-A"]);
        git(
            self.path(),
            &["commit", "-q", "--allow-empty", "-m", message],
        );
    }

    /// The ledger every worktree of the repository shares.
    fn ledger(&self) -> PathBuf {
        self.path()
            .join(".git")
            .join("anneal")
            .join("ledger.sqlite3")
    }

    /// Runs `anneal <args>` in the repository, expecting `code` and a quiet stderr, and returns
    /// the envelope.
    fn anneal(&self, args: &[&str], code: i32) -> Result<Value, Box<dyn Error>> {
        let output = anneal_in(self.path()).args(args).output()?;
        assert_eq!(
            output.status.code(),
            Some(code),
            "anneal {args:?}: {output:?}"
        );
        assert!(output.stderr.is_empty(), "anneal {args:?}: {output:?}");
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn build(&self) -> Result<Value, Box<dyn Error>> {
        self.anneal(&["mode", "build", "--plan", PLAN, "--tree", TREE], 0)
    }

    fn confirm(&self) -> Result<Value, Box<dyn Error>> {
        self.anneal(&["grill", "confirm", "--tree", TREE, "--quote", QUOTE], 0)
    }

    fn approve(&self) -> Result<Value, Box<dyn Error>> {
        self.anneal(&["plan", "approve", PLAN], 0)
    }

    /// Every precondition of build mode met: grill confirmed, plan approved, build entered on
    /// a clean tree.
    fn ready(&self) -> TestResult {
        self.confirm()?;
        self.approve()?;
        self.build()?;
        Ok(())
    }

    /// Breaks the spec's lint: a required section renamed.
    fn break_spec(&self) -> TestResult {
        let path = self.path().join(SPEC);
        let text = std::fs::read_to_string(&path)?;
        std::fs::write(&path, text.replace("## Open questions", "## Questions"))?;
        Ok(())
    }

    /// The `PreToolUse` input for `tool` writing `file_path`.
    fn input(&self, tool: &str, file_path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
        input_from(self.path(), tool, file_path)
    }

    /// Runs the hook on a Write of `rel` (relative to the repository root), with Claude Code's
    /// working directory, and the hook's, at `cwd`.
    fn write_from(&self, cwd: &Path, rel: &str) -> Result<Output, Box<dyn Error>> {
        let stdin = input_from(cwd, "Write", &self.path().join(rel))?;
        hook_in(cwd, &stdin)
    }

    /// Runs the hook on a Write of `rel` (relative to the repository root).
    fn write_to(&self, rel: &str) -> Result<Output, Box<dyn Error>> {
        self.hook(&self.input("Write", &self.path().join(rel))?)
    }

    /// Runs `anneal hook preconditions --fail-closed` in the repository with `stdin`.
    fn hook(&self, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
        hook_in(self.path(), stdin)
    }
}

/// The `PreToolUse` input for `tool` writing `file_path`, with Claude Code's working directory
/// at `cwd`.
fn input_from(cwd: &Path, tool: &str, file_path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(serde_json::to_vec(&json!({
        "session_id": "s-it",
        "transcript_path": "/dev/null",
        "cwd": cwd,
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": {"file_path": file_path, "content": "x"},
        "tool_use_id": "toolu_1",
    }))?)
}

/// Runs `anneal hook preconditions --fail-closed` in `cwd` with `stdin`.
fn hook_in(cwd: &Path, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
    let mut child = anneal_in(cwd)
        .args(["hook", "preconditions", "--fail-closed"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin is piped")?;
    input.write_all(stdin)?;
    drop(input);
    Ok(child.wait_with_output()?)
}

impl Repo {
    /// Appends a `repro_red` event, as `anneal repro record` will.
    fn repro(&self) -> TestResult {
        self.anneal(
            &[
                "ledger",
                "append",
                r#"{"type": "repro_red", "task": "bug-1", "test": "cargo test parse"}"#,
            ],
            0,
        )?;
        Ok(())
    }

    /// The `gate_blocked` events, through `anneal ledger query`.
    fn blocked(&self) -> Result<Vec<Value>, Box<dyn Error>> {
        let envelope = self.anneal(&["ledger", "query", "--type", "gate_blocked"], 0)?;
        Ok(envelope
            .pointer("/data/events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }
}

/// Asserts the hook allowed silently: exit 0, nothing on stdout or stderr.
fn assert_allowed(output: &Output, what: &str) {
    assert_eq!(output.status.code(), Some(0), "{what}: {output:?}");
    assert!(output.stdout.is_empty(), "{what}: silent allow: {output:?}");
    assert!(
        output.stderr.is_empty(),
        "{what}: nothing on stderr: {output:?}"
    );
}

/// The deny reason, after checking the deny decision's shape: exit 0, one JSON line on stdout,
/// nothing on stderr, a reason that is one `ERROR` line.
fn denial(output: &Output) -> Result<String, Box<dyn Error>> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "a decision exits 0: {output:?}"
    );
    assert!(output.stderr.is_empty(), "nothing on stderr: {output:?}");
    let decision: Value = serde_json::from_slice(&output.stdout)?;
    let specific = decision
        .get("hookSpecificOutput")
        .ok_or("hookSpecificOutput")?;
    assert_eq!(
        specific.get("hookEventName"),
        Some(&json!("PreToolUse")),
        "a PreToolUse decision"
    );
    assert_eq!(
        specific.get("permissionDecision"),
        Some(&json!("deny")),
        "a deny"
    );
    let reason = specific
        .get("permissionDecisionReason")
        .and_then(Value::as_str)
        .ok_or("a reason")?;
    assert!(reason.starts_with("ERROR "), "one ERROR line: {reason}");
    assert!(!reason.contains('\n'), "one line: {reason}");
    Ok(reason.to_owned())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn with_no_mode_an_edit_is_allowed_silently_and_no_ledger_is_created() -> TestResult {
    let repo = Repo::new()?;

    let output = repo.write_to(SOURCE)?;

    assert_allowed(&output, "no mode");
    assert!(!repo.ledger().exists(), "the hook never creates a ledger");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn with_a_ledger_but_no_mode_for_the_worktree_an_edit_is_allowed() -> TestResult {
    let repo = Repo::new()?;
    repo.confirm()?;

    let output = repo.write_to(SOURCE)?;

    assert_allowed(&output, "a ledger without mode_set");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn mode_build_records_the_mode_with_the_plan_and_tree_relative_to_the_root() -> TestResult {
    let repo = Repo::new()?;
    std::fs::create_dir(repo.path().join("src").join("sub"))?;

    let output = anneal_in(&repo.path().join("src").join("sub"))
        .args(["mode", "build", "--plan", "../../define/plan.md", "--tree"])
        .arg(
            Path::new("..")
                .join("..")
                .join("define")
                .join("design-tree.json"),
        )
        .output()?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    for (pointer, expected) in [
        ("/data/mode", json!("build")),
        ("/data/clean", json!(true)),
        ("/data/plan", json!(PLAN)),
        ("/data/tree", json!(TREE)),
    ] {
        assert_eq!(envelope.pointer(pointer), Some(&expected), "{pointer}");
    }
    let events = repo.anneal(&["ledger", "query", "--type", "mode_set"], 0)?;
    let event = events
        .pointer("/data/events/0/event")
        .ok_or("one mode_set")?;
    assert_eq!(event.get("plan"), Some(&json!(PLAN)), "{event}");
    assert_eq!(event.get("tree"), Some(&json!(TREE)), "{event}");
    assert!(
        event.get("worktree").and_then(Value::as_str).is_some(),
        "tagged with the worktree: {event}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn mode_build_on_a_dirty_tree_records_clean_false_with_a_finding_and_exits_0() -> TestResult {
    let repo = Repo::new()?;
    repo.write("notes.rs", "untracked\n")?;

    let envelope = repo.build()?;

    assert_eq!(
        envelope.pointer("/data/clean"),
        Some(&json!(false)),
        "recorded as not clean"
    );
    assert_eq!(
        envelope.pointer("/findings/0/key"),
        Some(&json!("mode/dirty")),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn mode_build_refuses_a_plan_that_is_not_a_file() -> TestResult {
    let repo = Repo::new()?;

    let output = anneal_in(repo.path())
        .args([
            "mode",
            "build",
            "--plan",
            "define/missing.md",
            "--tree",
            TREE,
        ])
        .output()?;

    assert_eq!(output.status.code(), Some(2), "a usage error: {output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert!(!repo.ledger().exists(), "nothing recorded");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_denies_a_source_edit_while_the_spec_fails_its_lint_then_allows_it() -> TestResult {
    let repo = Repo::new()?;
    repo.break_spec()?;
    repo.commit("broken spec");
    repo.ready()?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.starts_with("ERROR build mode: "), "{reason}");
    assert!(reason.contains(SPEC), "names the spec: {reason}");
    assert!(
        reason.contains(&format!("`anneal lint spec {SPEC}`")),
        "names the command: {reason}"
    );
    std::fs::copy(fixture().join("spec.md"), repo.path().join(SPEC))?;
    assert_allowed(&repo.write_to(SOURCE)?, "spec fixed");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_denies_a_source_edit_when_the_plan_links_no_spec_file() -> TestResult {
    let repo = Repo::new()?;
    std::fs::remove_file(repo.path().join(SPEC))?;
    repo.commit("no spec");
    // `plan approve` refuses a plan whose spec is missing; the spec is checked first anyway.
    repo.confirm()?;
    repo.build()?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.contains("spec"), "{reason}");
    assert!(reason.contains("`anneal lint spec"), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_denies_a_source_edit_until_the_tree_is_confirmed() -> TestResult {
    let repo = Repo::new()?;
    repo.approve()?;
    repo.build()?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.starts_with("ERROR build mode: "), "{reason}");
    assert!(
        reason.contains(&format!("`anneal grill confirm --tree {TREE} --quote")),
        "names the command: {reason}"
    );
    repo.confirm()?;
    assert_allowed(&repo.write_to(SOURCE)?, "tree confirmed");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_denies_a_source_edit_when_the_tree_changed_since_its_confirmation() -> TestResult {
    let repo = Repo::new()?;
    repo.ready()?;
    let path = repo.path().join(TREE);
    let text = std::fs::read_to_string(&path)?;
    std::fs::write(&path, format!("{text}\n"))?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.contains("`anneal grill confirm"), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_entered_on_a_dirty_tree_denies_until_entered_again_clean() -> TestResult {
    let repo = Repo::new()?;
    repo.confirm()?;
    repo.approve()?;
    repo.write("notes.rs", "untracked\n")?;
    repo.build()?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.contains("not clean"), "{reason}");
    assert!(
        reason.contains(&format!("`anneal mode build --plan {PLAN} --tree {TREE}`")),
        "names the command: {reason}"
    );
    std::fs::remove_file(repo.path().join("notes.rs"))?;
    repo.build()?;
    assert_allowed(&repo.write_to(SOURCE)?, "entered clean");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_denies_a_source_edit_until_the_plan_is_approved() -> TestResult {
    let repo = Repo::new()?;
    repo.confirm()?;
    repo.build()?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.contains("not approved"), "{reason}");
    assert!(
        reason.contains(&format!("`anneal plan approve {PLAN}`")),
        "names the command: {reason}"
    );
    repo.approve()?;
    assert_allowed(&repo.write_to(SOURCE)?, "plan approved");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_denies_a_source_edit_when_the_plan_changed_since_its_approval() -> TestResult {
    let repo = Repo::new()?;
    repo.ready()?;
    let path = repo.path().join(PLAN);
    let text = std::fs::read_to_string(&path)?;
    std::fs::write(&path, format!("{text}\n"))?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.contains("changed since it was approved"), "{reason}");
    assert!(
        reason.contains(&format!("`anneal plan approve {PLAN}`")),
        "{reason}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_denial_appends_gate_blocked_naming_the_precondition() -> TestResult {
    let repo = Repo::new()?;
    repo.confirm()?;
    repo.build()?;

    denial(&repo.write_to(SOURCE)?)?;

    let blocked = repo.blocked()?;
    let [recorded] = blocked.as_slice() else {
        return Err(format!("one gate_blocked, got {blocked:?}").into());
    };
    let event = recorded.get("event").ok_or("the stored event")?;
    assert_eq!(
        event.get("point"),
        Some(&json!("preconditions.plan")),
        "{event}"
    );
    assert_eq!(event.get("actor"), Some(&json!("hook")), "{event}");
    assert_eq!(event.get("session_id"), Some(&json!("s-it")), "{event}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn test_files_and_define_artifacts_are_allowed_while_every_precondition_fails() -> TestResult {
    let repo = Repo::new()?;
    repo.write("notes.rs", "untracked\n")?;
    repo.build()?;
    let outside = tempfile::tempdir()?;

    for rel in [
        "tests/cli.rs",
        "src/parser_test.go",
        PLAN,
        SPEC,
        TREE,
        ".temper/plans/x.rs",
        ".anneal/state.json",
        "tmp/x.rs",
        "scratch/y.py",
        "docs/notes.md",
        "NOTES.txt",
    ] {
        assert_allowed(&repo.write_to(rel)?, rel);
    }
    let elsewhere = repo.input("Write", &outside.path().join("main.rs"))?;
    assert_allowed(&repo.hook(&elsewhere)?, "outside the worktree");
    denial(&repo.write_to(SOURCE)?)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn an_edit_is_judged_like_a_write() -> TestResult {
    let repo = Repo::new()?;
    repo.build()?;

    let input = repo.input("Edit", &repo.path().join(SOURCE))?;

    denial(&repo.hook(&input)?)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_path_climbing_out_of_a_test_directory_is_still_source() -> TestResult {
    let repo = Repo::new()?;
    repo.build()?;

    let reason = denial(&repo.write_to("tests/../src/main.rs")?)?;

    assert!(reason.starts_with("ERROR build mode: "), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn mode_off_lifts_the_gate() -> TestResult {
    let repo = Repo::new()?;
    repo.build()?;
    denial(&repo.write_to(SOURCE)?)?;

    repo.anneal(&["mode", "off"], 0)?;

    assert_allowed(&repo.write_to(SOURCE)?, "mode off");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn bad_input_is_denied() -> TestResult {
    let repo = Repo::new()?;

    for stdin in [
        &b"not json"[..],
        b"{}",
        br#"{"tool_name": "Write", "tool_input": {}}"#,
    ] {
        let reason = denial(&repo.hook(stdin)?)?;
        assert!(reason.contains("input"), "{reason}");
    }
    assert!(!repo.ledger().exists(), "no ledger created for the record");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_ledger_that_cannot_be_opened_denies_rather_than_reading_as_no_mode() -> TestResult {
    let repo = Repo::new()?;
    std::fs::create_dir_all(repo.ledger())?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.contains("the ledger cannot be read"), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_mode_recorded_without_a_plan_denies_source_edits() -> TestResult {
    let repo = Repo::new()?;
    // The worktree key exactly as `anneal mode` writes it, read back from a real entry.
    let entered = repo.anneal(&["mode", "diagnose"], 0)?;
    let worktree = entered
        .pointer("/data/worktree")
        .cloned()
        .ok_or("mode reports its worktree")?;
    let event = json!({"type": "mode_set", "mode": "build", "clean": true, "worktree": worktree});
    repo.anneal(&["ledger", "append", &event.to_string()], 0)?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.contains("`anneal mode build --plan"), "{reason}");
    assert_allowed(&repo.write_to("tests/cli.rs")?, "a test file");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn another_tool_is_allowed() -> TestResult {
    let repo = Repo::new()?;
    repo.build()?;

    let input = repo.input("Read", &repo.path().join(SOURCE))?;

    assert_allowed(&repo.hook(&input)?, "Read");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn diagnose_mode_denies_a_source_edit_until_a_repro_is_recorded_after_entry() -> TestResult {
    let repo = Repo::new()?;
    repo.repro()?;
    repo.anneal(&["mode", "diagnose"], 0)?;

    let reason = denial(&repo.write_to(SOURCE)?)?;

    assert!(reason.starts_with("ERROR diagnose mode: "), "{reason}");
    assert!(reason.contains("`anneal repro record -- "), "{reason}");
    assert_allowed(&repo.write_to("tests/cli.rs")?, "a test while diagnosing");
    repo.repro()?;
    assert_allowed(&repo.write_to(SOURCE)?, "repro recorded after entry");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn entering_diagnose_again_needs_a_new_repro() -> TestResult {
    let repo = Repo::new()?;
    repo.anneal(&["mode", "diagnose"], 0)?;
    repo.repro()?;
    assert_allowed(&repo.write_to(SOURCE)?, "first session");

    repo.anneal(&["mode", "diagnose"], 0)?;

    denial(&repo.write_to(SOURCE)?)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_write_into_a_gated_worktree_from_a_cwd_outside_any_repository_is_denied() -> TestResult {
    let repo = Repo::new()?;
    repo.build()?;
    let elsewhere = tempfile::tempdir()?;

    let reason = denial(&repo.write_from(elsewhere.path(), SOURCE)?)?;

    assert!(reason.starts_with("ERROR build mode: "), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_write_into_a_gated_worktree_from_a_sibling_linked_worktree_is_denied() -> TestResult {
    let repo = Repo::new()?;
    let siblings = tempfile::tempdir()?;
    let sibling = siblings.path().join("sibling");
    let sibling_arg = sibling.to_str().ok_or("a UTF-8 temporary path")?;
    git(
        repo.path(),
        &["worktree", "add", "-q", "-b", "sibling", sibling_arg],
    );
    repo.build()?;
    let own = input_from(&sibling, "Write", &sibling.join(SOURCE))?;
    assert_allowed(
        &hook_in(&sibling, &own)?,
        "the sibling has no mode of its own",
    );

    let reason = denial(&repo.write_from(&sibling, SOURCE)?)?;

    assert!(reason.starts_with("ERROR build mode: "), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn mode_takes_no_store_flag_and_records_nothing_when_given_one() -> TestResult {
    let repo = Repo::new()?;
    let store = repo.path().join("elsewhere.sqlite3");

    let output = anneal_in(repo.path())
        .args(["mode", "diagnose", "--store"])
        .arg(&store)
        .output()?;

    assert_eq!(output.status.code(), Some(2), "a usage error: {output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout: {output:?}");
    assert!(!store.exists(), "no ledger at the flag's path");
    assert!(!repo.ledger().exists(), "nothing recorded");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_spec_line_pointing_at_source_does_not_exempt_that_source() -> TestResult {
    let repo = Repo::new()?;
    repo.ready()?;
    let plan = repo.path().join(PLAN);
    let text = std::fs::read_to_string(&plan)?;
    std::fs::write(&plan, text.replace("(spec.md)", "(../src/lib.rs)"))?;

    let reason = denial(&repo.write_to("src/lib.rs")?)?;

    assert!(reason.starts_with("ERROR build mode: "), "{reason}");
    Ok(())
}

#[test]
#[cfg(any(windows, target_os = "macos"))]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_mode_entered_through_one_letter_case_gates_edits_through_another() -> TestResult {
    let repo = Repo::new()?;
    let shouted = PathBuf::from(repo.path().to_string_lossy().to_uppercase());

    let output = anneal_in(&shouted)
        .args(["mode", "build", "--plan", PLAN, "--tree", TREE])
        .output()?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let reason = denial(&repo.write_to(SOURCE)?)?;
    assert!(reason.starts_with("ERROR build mode: "), "{reason}");
    Ok(())
}
