//! The edges of `anneal status`: the text form's flattening, cutting and wording, the
//! no-ledger text form, a ledger read outside any git repository, and a corrupt source
//! outside the sections asked for.

use std::path::Path;

use anneal_ledger::LedgerEvent;
use anneal_ledger::event::{GateBlocked, Mode, ModeSet, Verdict};
use clap::Parser;
use serde_json::{Value, json};

use super::{Fixture, append, build_mode};
use crate::cli::{Cli, Command};
use crate::context;
use crate::status::text::verdict_name;
use crate::test_repo::Setup;

/// Appends a `gate_blocked` for `point` with `reason` at `ts`.
fn block(fixture: &Fixture, point: &str, reason: &str, ts: &str) -> Setup<u64> {
    let gate = LedgerEvent::GateBlocked(GateBlocked {
        point: point.to_owned(),
        reason: reason.to_owned(),
    });
    append(&fixture.store()?, gate, None, ts)
}

/// Runs `anneal status --json <args>` in `cwd` instead of the repository.
fn json_in(fixture: &Fixture, cwd: &Path, args: &[&str]) -> Setup<(u8, Value)> {
    let command_line = ["anneal", "status", "--json"].iter().chain(args);
    let cli = Cli::try_parse_from(command_line)?;
    let Some(Command::Status(status)) = cli.command else {
        return Err("not the status command".into());
    };
    let mut out = Vec::new();
    let code = crate::status::run(&status, cwd, |var| fixture.vars.get(var).cloned(), &mut out)?;
    Ok((code.value(), serde_json::from_slice(&out)?))
}

#[test]
fn the_text_form_flattens_line_breaks_and_cuts_a_line_at_200_characters() -> Setup<()> {
    let fixture = Fixture::new()?;
    block(&fixture, "guard.first", "first", "2026-10-19T10:00:01Z")?;
    let reason = format!("line one\r\nline two\n{}", "x".repeat(250));
    block(&fixture, "guard.rm-rf", &reason, "2026-10-19T10:00:02Z")?;

    let (code, text) = fixture.status(&["--fields", "gates"])?;

    assert_eq!(code, 0, "{text}");
    let shown = "gates: 2 recent blocks, newest guard.rm-rf (#2): line one  line two ";
    let expected = format!("{shown}{}\n", "x".repeat(200 - shown.chars().count()));
    assert_eq!(
        text, expected,
        "CR and LF are spaces, and the line stops at its 200th character"
    );
    Ok(())
}

#[test]
fn the_text_form_without_a_ledger_exits_2_with_the_problem_line() -> Setup<()> {
    let fixture = Fixture::new()?;

    let (code, text) = fixture.status(&[])?;

    assert_eq!(code, 2, "no ledger: {text}");
    assert_eq!(
        text,
        "problems: no ledger exists yet: nothing has been recorded here. fix: run an anneal \
         command that records, such as `anneal mode build`\n",
        "the only line is the finding"
    );
    assert!(!fixture.store_path().exists(), "no ledger was created");
    Ok(())
}

#[test]
fn outside_a_repository_only_the_ledger_wide_sections_are_read() -> Setup<()> {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let mode = LedgerEvent::ModeSet(ModeSet {
        mode: Mode::Diagnose,
        clean: true,
        plan: None,
        tree: None,
    });
    append(&store, mode, Some("elsewhere"), "2026-10-19T10:00:01Z")?;
    block(&fixture, "guard.rm-rf", "rm -rf /", "2026-10-19T10:00:02Z")?;
    let outside = tempfile::tempdir()?;

    let (code, envelope) = json_in(&fixture, outside.path(), &[])?;

    assert_eq!(code, 0, "a ledger exists: {envelope}");
    assert_eq!(
        envelope["data"],
        json!({
            "schema": 1,
            "gates": [
                {"id": "guard.rm-rf", "kind": "gate_blocked", "reason": "rm -rf /", "event": 2,
                 "ts": "2026-10-19T10:00:02Z"},
            ],
            "ledger_tail": [
                {"event": 1, "type": "mode_set", "summary": "mode_set diagnose", "actor": "cli",
                 "ts": "2026-10-19T10:00:01Z"},
                {"event": 2, "type": "gate_blocked", "summary": "gate_blocked point:guard.rm-rf",
                 "actor": "cli",
                 "ts": "2026-10-19T10:00:02Z"},
            ],
        }),
        "no worktree, so no phase, plan, tasks or verdict; no Clef or context reading"
    );
    assert_eq!(
        envelope["findings"],
        json!([]),
        "being outside a repository is not a problem"
    );
    Ok(())
}

#[test]
fn a_corrupt_context_reading_is_not_read_for_other_sections() -> Setup<()> {
    let fixture = Fixture::new()?;
    build_mode(&fixture.store()?, &fixture.key()?)?;
    std::fs::create_dir_all(fixture.home())?;
    std::fs::write(
        context::in_home(&fixture.home(), context::USAGE_FILE),
        "not json",
    )?;

    let (code, envelope) = fixture.json(&["--fields", "phase"])?;

    assert_eq!(code, 0, "{envelope}");
    assert_eq!(
        envelope["data"]["phase"]["mode"],
        json!("build"),
        "the phase is read: {envelope}"
    );
    assert_eq!(
        envelope["findings"],
        json!([]),
        "context was not asked for, so its file is not read"
    );
    Ok(())
}

#[test]
fn one_recent_block_is_singular() -> Setup<()> {
    let fixture = Fixture::new()?;
    block(&fixture, "guard.rm-rf", "rm -rf /", "2026-10-19T10:00:01Z")?;

    let (_, text) = fixture.status(&["--fields", "gates"])?;

    assert_eq!(
        text, "gates: 1 recent block, newest guard.rm-rf (#1): rm -rf /\n",
        "one block, not one blocks"
    );
    Ok(())
}

#[test]
fn the_text_form_shows_control_and_unprintable_characters_as_spaces() -> Setup<()> {
    let fixture = Fixture::new()?;
    let reason = "\u{1b}[31mred\u{1b}[0m\tand\u{2028}more\u{200b}end\u{7f}";
    block(&fixture, "guard.rm-rf", reason, "2026-10-19T10:00:01Z")?;

    let (_, text) = fixture.status(&["--fields", "gates"])?;

    assert_eq!(
        text, "gates: 1 recent block, newest guard.rm-rf (#1):  [31mred [0m and more end \n",
        "ESC, tab, DEL, U+2028 and a zero-width space cannot recolour or break the terminal"
    );
    Ok(())
}

#[test]
fn verdict_names_are_the_ledger_serde_names() -> Setup<()> {
    let all = [
        Verdict::TestsVerified,
        Verdict::Reviewed,
        Verdict::Approved,
        Verdict::ChangesRequested,
        Verdict::NoiseOnly,
    ];
    for verdict in all {
        assert_eq!(
            json!(verdict_name(verdict)),
            serde_json::to_value(verdict)?,
            "the text form names {verdict:?} as the ledger writes it"
        );
    }
    Ok(())
}
