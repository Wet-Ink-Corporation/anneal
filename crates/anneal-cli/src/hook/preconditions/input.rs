//! The hook's input: the fields of Claude Code's `PreToolUse` JSON it uses, the gated tools,
//! and the files a call names ([`ToolInput::paths`]).

use std::path::PathBuf;

use serde::Deserialize;

use super::{Check, Denial};

/// Most distinct paths one call may name; a call naming more is denied as bad input.
pub(super) const MAX_PATHS: usize = 16;

/// The tools whose calls are judged; any other is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    Edit,
    Write,
    MultiEdit,
    NotebookEdit,
}

impl Tool {
    /// The gated tool named `name`, `None` for any other.
    pub(super) fn named(name: &str) -> Option<Self> {
        match name {
            "Edit" => Some(Self::Edit),
            "Write" => Some(Self::Write),
            "MultiEdit" => Some(Self::MultiEdit),
            "NotebookEdit" => Some(Self::NotebookEdit),
            _ => None,
        }
    }

    /// The `tool_input` field naming the file the tool writes, which its input must have.
    const fn path_field(self) -> &'static str {
        match self {
            Self::Edit | Self::Write | Self::MultiEdit => "file_path",
            Self::NotebookEdit => "notebook_path",
        }
    }
}

/// The fields of Claude Code's `PreToolUse` input the hook uses; the rest are ignored.
#[derive(Debug, Deserialize)]
pub(super) struct Input {
    #[serde(default)]
    pub(super) session_id: Option<String>,
    /// Where the ledger and the worktree are found, and relative paths resolved; the process's
    /// working directory when absent.
    #[serde(default)]
    pub(super) cwd: Option<PathBuf>,
    pub(super) tool_name: String,
    #[serde(default, rename = "tool_input")]
    pub(super) tool: ToolInput,
}

/// The gated tools' input: every field that names a file; other fields are ignored.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ToolInput {
    /// The file of Edit, Write and `MultiEdit`.
    #[serde(default)]
    file_path: Option<PathBuf>,
    /// The notebook of `NotebookEdit`.
    #[serde(default)]
    notebook_path: Option<PathBuf>,
    /// The edits of `MultiEdit`, any of which may name a file of its own.
    #[serde(default)]
    edits: Option<Vec<EditInput>>,
}

/// One of the edits of `MultiEdit`; only a file it names is used.
#[derive(Debug, Deserialize)]
struct EditInput {
    #[serde(default)]
    file_path: Option<PathBuf>,
}

impl ToolInput {
    /// Every distinct path the call to `tool` (named `tool_name`) names, the one the tool
    /// writes through first; the denial when that one is missing or there are more than
    /// [`MAX_PATHS`].
    pub(super) fn paths(self, tool: Tool, tool_name: &str) -> Result<Vec<PathBuf>, Denial> {
        let (primary, other) = match tool {
            Tool::Edit | Tool::Write | Tool::MultiEdit => (self.file_path, self.notebook_path),
            Tool::NotebookEdit => (self.notebook_path, self.file_path),
        };
        let Some(primary) = primary else {
            return Err(Denial::new(
                Check::BadInput,
                format!(
                    "preconditions: the hook input cannot be read: the {tool_name} input has no \
                     tool_input.{}",
                    tool.path_field()
                ),
            ));
        };
        let edits = self
            .edits
            .into_iter()
            .flatten()
            .filter_map(|edit| edit.file_path);
        let mut paths = vec![primary];
        for path in other.into_iter().chain(edits) {
            if paths.contains(&path) {
                continue;
            }
            if paths.len() == MAX_PATHS {
                return Err(Denial::new(
                    Check::BadInput,
                    format!(
                        "preconditions: the hook input cannot be read: the {tool_name} input \
                         names more than {MAX_PATHS} files"
                    ),
                ));
            }
            paths.push(path);
        }
        Ok(paths)
    }
}
