//! `.anneal/config.json`: the repository's `check:fast` command.
//!
//! ```json
//! {"check_fast": ["cargo", "check", "--quiet"]}
//! ```
//!
//! `check_fast` is an argv array, run without a shell. Other keys belong to other readers and
//! are ignored here.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::Deserialize;

/// The config file, relative to the worktree root.
pub(crate) const CONFIG_PATH: &str = ".anneal/config.json";

/// Most bytes of the config read.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

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
    /// The file is over [`MAX_CONFIG_BYTES`].
    #[error("{CONFIG_PATH} is larger than {MAX_CONFIG_BYTES} bytes")]
    TooLarge,
    /// Not a JSON object, or `check_fast` is not an array of strings.
    #[error("{CONFIG_PATH} is not a config object with check_fast as an array of strings")]
    Json(#[source] serde_json::Error),
    /// `check_fast` is an empty array: there is no program to run.
    #[error("{CONFIG_PATH}: check_fast is empty; give the program and its arguments")]
    EmptyCheckFast,
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
    let file = match File::open(root.join(CONFIG_PATH)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigError::Read(error)),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(ConfigError::Read)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_CONFIG_BYTES) {
        return Err(ConfigError::TooLarge);
    }
    check_fast(&String::from_utf8_lossy(&bytes))
}
