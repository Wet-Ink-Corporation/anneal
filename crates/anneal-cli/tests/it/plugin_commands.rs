//! Every `anneal …` command a plugin file names is a real subcommand (Move 2 prep, CLI first):
//! a skill that tells the model to run a command the binary does not have fails here, not in a
//! session.
//!
//! Three sources are read: backtick spans that start with `anneal ` in the prose of the plugins'
//! text files, lines of fenced code blocks that start with `anneal ` (after an optional `$ `
//! prompt), and the exec-form `args` of every `hooks.json` entry whose `command` is the `anneal` launcher.
//! The first word must be a top-level command; when that command has subcommands, the second
//! word must be one of them unless it is an option (`-…`) or a placeholder (`<…>`).

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use anneal_cli::cli::Cli;
use clap::CommandFactory;
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

/// Plugin files that tell the model or Claude Code which command to run: skills and their
/// references, playbooks, hook manifests. The cockpit's TypeScript is left out: its backticks
/// are template literals (`anneal cockpit: …` messages), not commands for the model.
const TEXT_EXTENSIONS: [&str; 2] = ["md", "json"];

#[test]
fn an_unknown_command_is_reported() {
    assert_eq!(
        unknown_commands("Run `anneal frobnicate --x` first."),
        vec!["anneal frobnicate".to_owned()]
    );
}

#[test]
fn an_unknown_subcommand_is_reported() {
    assert_eq!(
        unknown_commands("`anneal plan frobnicate <file>` then `anneal plan check <file>`"),
        vec!["anneal plan frobnicate".to_owned()]
    );
}

#[test]
fn options_placeholders_and_leaf_arguments_are_not_subcommands() {
    let text = "`anneal calibrate --apply` `anneal hook <id>` `anneal task done -- cargo test` \
                `anneal calibrate unavailable: ${held.reason}` `anneal`";
    assert_eq!(unknown_commands(text), Vec::<String>::new());
}

#[test]
fn a_command_in_a_fenced_block_is_checked() {
    let text = "Run:\n\n```bash\n$ anneal frobnicate --x\nanneal task done T3 -- cargo test\n```\n\n\
                then `anneal plan nope`.";
    assert_eq!(
        unknown_commands(text),
        vec![
            "anneal frobnicate".to_owned(),
            "anneal plan nope".to_owned()
        ]
    );
}

#[test]
fn a_hook_entry_naming_an_unknown_hook_is_reported() -> TestResult {
    let manifest: Value = serde_json::from_str(
        r#"{"hooks":{"Stop":[{"hooks":[
            {"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/libexec/anneal","args":["hook","frobnicate"]},
            {"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/libexec/anneal","args":["hook","status"]},
            {"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/libexec/other","args":["nope"]}
        ]}]}}"#,
    )?;
    assert_eq!(
        unknown_hook_commands(&manifest),
        vec!["anneal hook frobnicate".to_owned()]
    );
    Ok(())
}

#[test]
fn every_command_the_plugins_name_exists() -> TestResult {
    let mut files = Vec::new();
    for dir in ["plugin", "plugin-cockpit"] {
        collect_text_files(&repo().join(dir), &mut files)?;
    }
    assert!(
        files.iter().any(|f| f.ends_with("SKILL.md")),
        "the walk found no skills under {}",
        repo().display()
    );

    let mut unknown = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file)?;
        let mut found = unknown_commands(&text);
        if file.file_name().is_some_and(|n| n == "hooks.json") {
            found.extend(unknown_hook_commands(&serde_json::from_str(&text)?));
        }
        unknown.extend(
            found
                .into_iter()
                .map(|c| format!("{}: {c}", file.display())),
        );
    }
    assert_eq!(
        unknown,
        Vec::<String>::new(),
        "commands the CLI does not define"
    );
    Ok(())
}

/// Each `anneal …` command in `text` whose command path the CLI does not define, as the words
/// that failed (`anneal plan frobnicate`): first the lines of fenced code blocks that start
/// with `anneal ` (after an optional `$ ` prompt), then the inline backtick spans in the prose
/// outside the fences.
fn unknown_commands(text: &str) -> Vec<String> {
    let root = Cli::command();
    let mut fenced = Vec::new();
    let mut prose = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        } else if in_fence {
            let command = line.trim_start();
            let command = command.strip_prefix("$ ").unwrap_or(command);
            fenced.extend(command.strip_prefix("anneal "));
        } else {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    let inline = prose
        .split('`')
        .skip(1)
        .step_by(2)
        .filter_map(|span| span.strip_prefix("anneal "));
    fenced
        .into_iter()
        .chain(inline)
        .filter_map(|rest| unknown_path(&root, rest.split_whitespace()))
        .collect()
}

/// The exec-form `args` of each hook entry that runs the `anneal` launcher and names a command
/// path the CLI does not define.
fn unknown_hook_commands(manifest: &Value) -> Vec<String> {
    let root = Cli::command();
    let mut entries = Vec::new();
    hook_entries(manifest, &mut entries);
    entries
        .into_iter()
        .filter(|entry| {
            entry
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|c| c.ends_with("/libexec/anneal"))
        })
        .filter_map(|entry| {
            let args = entry.get("args").and_then(Value::as_array)?;
            unknown_path(&root, args.iter().filter_map(Value::as_str))
        })
        .collect()
}

/// Every JSON object that has a `command` key, found anywhere under `value`.
fn hook_entries<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.contains_key("command") {
                out.push(value);
            }
            map.values().for_each(|v| hook_entries(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| hook_entries(v, out)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// `None` when the words after `anneal` name a real command path; otherwise the failing words.
fn unknown_path<'a>(
    root: &clap::Command,
    mut words: impl Iterator<Item = &'a str>,
) -> Option<String> {
    let first = words.next()?;
    if is_not_a_name(first) {
        return None;
    }
    let Some(command) = root.find_subcommand(first) else {
        return Some(format!("anneal {first}"));
    };
    if !command.has_subcommands() {
        return None;
    }
    let second = words.next()?;
    if is_not_a_name(second) || command.find_subcommand(second).is_some() {
        return None;
    }
    Some(format!("anneal {first} {second}"))
}

/// An option, a placeholder or a template expression stands for a value, not a command name.
fn is_not_a_name(word: &str) -> bool {
    word.starts_with('-') || word.starts_with('<') || word.starts_with('$')
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The text files under `dir`, skipping `libexec/` (launchers and builds).
fn collect_text_files(dir: &Path, out: &mut Vec<PathBuf>) -> TestResult {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "libexec") {
                collect_text_files(&path, out)?;
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| TEXT_EXTENSIONS.contains(&e))
        {
            out.push(path);
        }
    }
    Ok(())
}
