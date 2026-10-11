//! The batch of tool calls a `PostToolBatch` input carries, and the files it edited.
//!
//! Claude Code 2.1.296 fires `PostToolBatch` once after every tool call of a batch has
//! resolved, with `tool_calls`: an array of `{tool_name, tool_input, tool_use_id,
//! tool_response}`. Only the file-editing tools make a batch worth checking.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

/// The tools that edit files, and the `tool_input` key naming the file.
const EDIT_TOOLS: [(&str, &str); 4] = [
    ("Edit", "file_path"),
    ("Write", "file_path"),
    ("MultiEdit", "file_path"),
    ("NotebookEdit", "notebook_path"),
];

/// Most changed files checked in one run; the rest are counted. They are passed to `anneal lint
/// terms` as arguments, so this keeps the command line well inside Windows' 32,767 characters.
pub(crate) const MAX_FILES: usize = 128;

/// The fields of the `PostToolBatch` input the hook uses; the rest are ignored.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Input {
    /// The session's working directory; the process's when absent.
    #[serde(default)]
    pub(crate) cwd: Option<PathBuf>,
    /// The batch; absent in an input that does not list it.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

/// One call of the batch. `tool_input` stays raw: its shape depends on the tool.
#[derive(Debug, Deserialize)]
struct ToolCall {
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
}

/// What the batch asks of the hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Batch {
    /// No call edited a file: nothing to check.
    NoEdits,
    /// The files the edit calls named, each once, in the order first named (as given: absolute
    /// or relative to the input's `cwd`), at most [`MAX_FILES`]; `omitted` counts the rest.
    Files {
        /// The files.
        files: Vec<String>,
        /// How many more there were.
        omitted: usize,
    },
    /// The input does not list its calls: the changed files come from git.
    Unlisted,
}

impl Input {
    /// The batch's edited files.
    pub(crate) fn batch(&self) -> Batch {
        let Some(calls) = &self.tool_calls else {
            return Batch::Unlisted;
        };
        let mut files: Vec<String> = Vec::new();
        let mut omitted = 0_usize;
        let mut edits = false;
        for call in calls {
            let Some((_, key)) = EDIT_TOOLS.iter().find(|(tool, _)| *tool == call.tool_name) else {
                continue;
            };
            edits = true;
            let Some(path) = call.tool_input.get(key).and_then(Value::as_str) else {
                continue;
            };
            if files.iter().any(|file| file == path) {
                continue;
            }
            if files.len() < MAX_FILES {
                // The batch owns the paths it hands on.
                files.push(path.to_owned());
            } else {
                omitted = omitted.saturating_add(1);
            }
        }
        if edits {
            Batch::Files { files, omitted }
        } else {
            Batch::NoEdits
        }
    }
}

/// The paths of `git status --porcelain=v1 -z --no-renames` output: one `XY path` record per
/// NUL-terminated entry.
pub(crate) fn porcelain(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter_map(|record| record.get(3..))
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect()
}
