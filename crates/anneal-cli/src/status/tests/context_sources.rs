//! `anneal status`'s `context` from both usage files: the status line's
//! (`context-usage.json`) and the mod's (`context-usage-mod.json`), the fresher winning, and
//! an unreadable one reported beside the other's reading (#160, weigh-in wi-6c304b).

use std::path::Path;

use serde_json::{Value, json};

use super::Fixture;
use crate::context::usage::WindowSource;
use crate::context::{self, Saved, Snapshot, Source};
use crate::test_repo::Setup;

/// Saves `used_tokens` of a 200,000-token window for session `session` as `source`, at `at_ms`.
fn save(home: &Path, session: &str, (source, used_tokens, at_ms): (Source, f64, u64)) -> Setup<()> {
    let reading = Snapshot {
        used_tokens: Some(used_tokens),
        window: 200_000,
        window_source: WindowSource::Model,
    };
    Ok(context::save(
        home,
        session,
        Saved::stamped(reading, source, at_ms),
    )?)
}

/// `anneal status --json --fields context`: the `context` section, or null when absent, and
/// the findings.
fn context_section(fixture: &Fixture) -> Setup<(Value, Value)> {
    let (code, mut envelope) = fixture.json(&["--fields", "context"])?;
    assert_eq!(code, 0, "{envelope}");
    let mut take = |pointer: &str| {
        envelope
            .pointer_mut(pointer)
            .map_or(Value::Null, Value::take)
    };
    let section = take("/data/context");
    Ok((section, take("/findings")))
}

#[test]
fn the_mod_s_snapshot_alone_is_the_context() -> Setup<()> {
    let fixture = Fixture::new()?;
    fixture.store()?;
    save(&fixture.home(), "s1", (Source::Mod, 150_000.0, 100))?;

    let (section, findings) = context_section(&fixture)?;

    assert_eq!(
        section,
        json!({"used_tokens": 150_000.0, "window": 200_000, "window_source": "model",
               "percent": 75.0, "level": "stop"}),
        "only the mod saved usage"
    );
    assert_eq!(findings, json!([]), "nothing unreadable");
    Ok(())
}

#[test]
fn the_fresher_source_s_snapshot_is_the_context_and_the_mod_s_on_a_tie() -> Setup<()> {
    let fixture = Fixture::new()?;
    fixture.store()?;
    let home = fixture.home();
    save(&home, "s1", (Source::Mod, 20_000.0, 100))?;
    save(&home, "s2", (Source::Statusline, 130_000.0, 200))?;

    let (status_line_newer, _) = context_section(&fixture)?;
    save(&home, "s1", (Source::Mod, 40_000.0, 200))?;
    let (tied, _) = context_section(&fixture)?;

    assert_eq!(
        status_line_newer["used_tokens"],
        json!(130_000.0),
        "the status line saved last: {status_line_newer}"
    );
    assert_eq!(
        tied["used_tokens"],
        json!(40_000.0),
        "same time: the mod's: {tied}"
    );
    Ok(())
}

#[test]
fn an_unreadable_mod_file_is_a_finding_beside_the_status_line_s_context() -> Setup<()> {
    let fixture = Fixture::new()?;
    fixture.store()?;
    let home = fixture.home();
    save(&home, "s1", (Source::Statusline, 120_000.0, 100))?;
    std::fs::write(context::in_home(&home, context::MOD_USAGE_FILE), "not json")?;

    let (section, findings) = context_section(&fixture)?;

    assert_eq!(
        section["percent"],
        json!(60.0),
        "the status line's reading is still shown: {section}"
    );
    assert_eq!(
        findings.as_array().map(Vec::len),
        Some(1),
        "one finding: {findings}"
    );
    assert_eq!(findings[0]["key"], json!("status/unreadable:context"));
    assert!(
        findings[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains(context::MOD_USAGE_FILE)),
        "it names the mod's file: {findings}"
    );
    Ok(())
}
