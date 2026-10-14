//! `.anneal/config.json`: the repository's `check:fast` command.
//!
//! ```json
//! {"check_fast": ["cargo", "check", "--quiet"]}
//! ```
//!
//! `check_fast` is an argv array, run without a shell. Other keys belong to other readers and
//! are ignored here. The file is read through `repo_config::read_text`, which the merge gate's
//! settings share.

use std::io;
use std::path::Path;

use serde::Deserialize;

use crate::repo_config::{self, CONFIG_PATH, FileError};

/// The fields this hook reads.
#[derive(Debug, Deserialize)]
struct ConfigDto {
    #[serde(default)]
    check_fast: Option<Vec<String>>,
}

/// Why the config cannot give a `check:fast` command.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigError {
    /// The file exists but cannot be read.
    #[error("{CONFIG_PATH} cannot be read")]
    Read(#[source] io::Error),
    /// The path is not a regular file (a directory, a symlink, a FIFO): never opened.
    #[error("{CONFIG_PATH} is not a regular file")]
    NotAFile,
    /// The file is over the config's size cap.
    #[error("{}", FileError::TooLarge)]
    TooLarge,
    /// Not a JSON object, or `check_fast` is not an array of strings.
    #[error("{CONFIG_PATH} is not a config object with check_fast as an array of strings")]
    Json(#[source] serde_json::Error),
    /// `check_fast` is an empty array: there is no program to run.
    #[error("{CONFIG_PATH}: check_fast is empty; give the program and its arguments")]
    EmptyCheckFast,
}

impl From<FileError> for ConfigError {
    fn from(error: FileError) -> Self {
        match error {
            FileError::Read(source) => Self::Read(source),
            FileError::NotAFile => Self::NotAFile,
            FileError::TooLarge => Self::TooLarge,
        }
    }
}

/// The `check:fast` argv of the config `text`; `None` when it sets none.
///
/// # Errors
/// The [`ConfigError`] when the config is not valid for it.
pub(crate) fn check_fast(text: &str) -> Result<Option<Vec<String>>, ConfigError> {
    let dto: ConfigDto = serde_json::from_str(text).map_err(ConfigError::Json)?;
    match dto.check_fast {
        Some(argv) if argv.is_empty() => Err(ConfigError::EmptyCheckFast),
        argv => Ok(argv),
    }
}

/// The `check:fast` argv of the config in the worktree `root`; `None` when there is no config
/// file or it sets none.
///
/// # Errors
/// The [`ConfigError`] when the file cannot be read or is not valid.
pub(crate) fn read(root: &Path) -> Result<Option<Vec<String>>, ConfigError> {
    match repo_config::read_text(root)? {
        Some(text) => check_fast(&text),
        None => Ok(None),
    }
}
