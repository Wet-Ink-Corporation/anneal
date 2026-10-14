//! `.anneal/config.json`, the repository's anneal settings: its bounded read (`read_text`),
//! shared with `hook::fast_check` (`check_fast`), and the merge gate's settings (`gate`):
//!
//! ```json
//! {"review": {"require_plugin": false},
//!  "ship": {"sensitive_globs": ["**/billing/**"]}}
//! ```
//!
//! `review.require_plugin` (default `false`): while false, a human `approved` verdict satisfies
//! `ship`'s review requirement; once true, only the review plugin's verdict does.
//! `ship.sensitive_globs` (default none): paths that force a full review, added to the
//! skip-rule's defaults (`ship::skip_rule::DEFAULT_SENSITIVE`), never replacing them. Each
//! reader ignores the other readers' keys, but `review` and `ship` are read strictly, so a
//! misspelt switch is an error rather than its default.
//!
//! `orchestrate.leads` (default `["orchestrate-lead"]`, at most 64): the subagent types the
//! brief hook lets spawn from inside a subagent (`orchestrate_leads`); a configured list
//! replaces the default, and `orchestrate` is read strictly too.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use serde::Deserialize;

use crate::glob::{Glob, GlobError};

/// The config file, relative to the worktree root.
pub(crate) const CONFIG_PATH: &str = ".anneal/config.json";

/// Most bytes of the config read.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Most globs `ship.sensitive_globs` may list (H-13).
pub(crate) const MAX_SENSITIVE_GLOBS: usize = 256;

/// Why the config file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// The file exists but cannot be read.
    #[error("{CONFIG_PATH} cannot be read")]
    Read(#[source] io::Error),
    /// The path is not a regular file (a directory, a symlink, a FIFO): never opened.
    #[error("{CONFIG_PATH} is not a regular file")]
    NotAFile,
    /// The file is over `MAX_CONFIG_BYTES` (64 KiB).
    #[error("{CONFIG_PATH} is larger than {MAX_CONFIG_BYTES} bytes")]
    TooLarge,
}

/// The text of the config in the worktree `root` (lossy UTF-8); `None` when there is none.
///
/// # Errors
/// The [`FileError`] when the path is not a regular file, cannot be read or is too large.
pub(crate) fn read_text(root: &Path) -> Result<Option<String>, FileError> {
    let path = root.join(CONFIG_PATH);
    // Checked before opening: a FIFO would block a hook, and a directory opens on unix only.
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(FileError::NotAFile),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(FileError::Read(error)),
    }
    let file = File::open(&path).map_err(FileError::Read)?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(FileError::Read)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_CONFIG_BYTES) {
        return Err(FileError::TooLarge);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// The merge gate's settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GateConfig {
    /// `review.require_plugin`: only the review plugin's verdict satisfies the review
    /// requirement.
    pub(crate) require_plugin: bool,
    /// `ship.sensitive_globs`: added to the skip-rule's default sensitive globs.
    pub(crate) sensitive_globs: Vec<Glob>,
}

/// The keys [`gate`] reads; the rest of the file belongs to other readers.
#[derive(Debug, Default, Deserialize)]
struct GateDto {
    #[serde(default)]
    review: ReviewDto,
    #[serde(default)]
    ship: ShipDto,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewDto {
    #[serde(default)]
    require_plugin: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShipDto {
    #[serde(default)]
    sensitive_globs: Vec<String>,
}

/// Why the merge gate's settings could not be read.
#[derive(Debug, thiserror::Error)]
pub enum GateConfigError {
    /// The file could not be read.
    #[error(transparent)]
    File(#[from] FileError),
    /// Not a JSON object, or `review` or `ship` is not as documented.
    #[error(
        "{CONFIG_PATH}: `review` must be {{\"require_plugin\": <bool>}} and `ship` \
         {{\"sensitive_globs\": [<glob>, ...]}}"
    )]
    Json(#[source] serde_json::Error),
    /// `ship.sensitive_globs` lists more than `MAX_SENSITIVE_GLOBS` (256).
    #[error("{CONFIG_PATH}: ship.sensitive_globs lists more than {MAX_SENSITIVE_GLOBS} globs")]
    TooManyGlobs,
    /// A glob in `ship.sensitive_globs` was refused.
    #[error("{CONFIG_PATH}: ship.sensitive_globs {glob:?}")]
    Glob {
        /// The glob as written.
        glob: String,
        /// Why it was refused.
        #[source]
        source: GlobError,
    },
}

/// The merge gate's settings from the config in the worktree `root`; the defaults when there
/// is no config file.
///
/// # Errors
/// The [`GateConfigError`] when the file cannot be read or its settings are not valid.
pub(crate) fn gate(root: &Path) -> Result<GateConfig, GateConfigError> {
    match read_text(root)? {
        Some(text) => gate_from_text(&text),
        None => Ok(GateConfig::default()),
    }
}

/// The merge gate's settings from the config `text`.
///
/// # Errors
/// As [`gate`], for the settings.
pub(crate) fn gate_from_text(text: &str) -> Result<GateConfig, GateConfigError> {
    let dto: GateDto = serde_json::from_str(text).map_err(GateConfigError::Json)?;
    if dto.ship.sensitive_globs.len() > MAX_SENSITIVE_GLOBS {
        return Err(GateConfigError::TooManyGlobs);
    }
    let sensitive_globs = dto
        .ship
        .sensitive_globs
        .into_iter()
        .map(|glob| match Glob::new(&glob) {
            Ok(compiled) => Ok(compiled),
            // The error owns the glob it names.
            Err(source) => Err(GateConfigError::Glob { glob, source }),
        })
        .collect::<Result<_, _>>()?;
    Ok(GateConfig {
        require_plugin: dto.review.require_plugin,
        sensitive_globs,
    })
}

/// The `agent_type`s that may spawn a subagent from inside a subagent when the config names
/// none (`orchestrate.leads`).
pub(crate) const DEFAULT_LEADS: [&str; 1] = ["orchestrate-lead"];

/// Most names `orchestrate.leads` may list (H-13).
pub(crate) const MAX_LEADS: usize = 64;

/// The key the brief hook reads; the rest of the file belongs to other readers.
#[derive(Debug, Default, Deserialize)]
struct LeadsDto {
    #[serde(default)]
    orchestrate: OrchestrateDto,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrchestrateDto {
    #[serde(default)]
    leads: Option<Vec<String>>,
}

/// Most bytes of one name in `orchestrate.leads` (H-13).
pub(crate) const MAX_LEAD_BYTES: usize = 128;

/// Why `orchestrate.leads` could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LeadsError {
    /// The file could not be read.
    #[error(transparent)]
    File(#[from] FileError),
    /// Not a JSON object, or `orchestrate` is not as documented.
    #[error("{CONFIG_PATH}: `orchestrate` must be {{\"leads\": [<agent type>, ...]}}")]
    Json(#[source] serde_json::Error),
    /// `orchestrate.leads` lists more than `MAX_LEADS` (64) names.
    #[error("{CONFIG_PATH}: orchestrate.leads lists more than {MAX_LEADS} names")]
    TooMany,
    /// A name in `orchestrate.leads` is not one an `agent_type` can be.
    #[error(
        "{CONFIG_PATH}: orchestrate.leads[{index}] {problem}; a lead is 1 to {MAX_LEAD_BYTES} \
         bytes of printable text"
    )]
    BadName {
        /// The name's place in the list, from 0.
        index: usize,
        /// What is wrong with it.
        problem: NameProblem,
    },
}

/// What is wrong with a name in `orchestrate.leads`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum NameProblem {
    /// It is empty.
    #[error("is empty")]
    Empty,
    /// It is longer than [`MAX_LEAD_BYTES`].
    #[error("is longer than {MAX_LEAD_BYTES} bytes")]
    TooLong,
    /// It holds a control character, such as a line break.
    #[error("holds a control character")]
    Control,
}

/// The orchestrate leads: the `agent_type`s allowed to spawn subagents from inside a subagent,
/// each 1 to [`MAX_LEAD_BYTES`] bytes with no control character, at most [`MAX_LEADS`]. An
/// empty list lets no subagent spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Leads {
    names: Vec<String>,
}

impl Leads {
    /// Whether `agent_type` is one of the leads (exactly, as Claude Code reports it).
    pub(crate) fn contains(&self, agent_type: &str) -> bool {
        self.names.iter().any(|name| name == agent_type)
    }

    /// The leads, in the configured order.
    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }

    /// [`DEFAULT_LEADS`].
    fn default_leads() -> Self {
        Self {
            names: DEFAULT_LEADS.map(str::to_owned).to_vec(),
        }
    }
}

impl TryFrom<Vec<String>> for Leads {
    type Error = LeadsError;

    fn try_from(names: Vec<String>) -> Result<Self, LeadsError> {
        if names.len() > MAX_LEADS {
            return Err(LeadsError::TooMany);
        }
        let problem = |name: &str| {
            if name.is_empty() {
                Some(NameProblem::Empty)
            } else if name.len() > MAX_LEAD_BYTES {
                Some(NameProblem::TooLong)
            } else if name.chars().any(char::is_control) {
                Some(NameProblem::Control)
            } else {
                None
            }
        };
        if let Some((index, problem)) = names
            .iter()
            .enumerate()
            .find_map(|(index, name)| problem(name).map(|problem| (index, problem)))
        {
            return Err(LeadsError::BadName { index, problem });
        }
        Ok(Self { names })
    }
}

/// The names, joined with `, `.
impl fmt::Display for Leads {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.names().join(", "))
    }
}

/// The orchestrate leads from the config in the worktree `root`. [`DEFAULT_LEADS`] when the
/// file or the key is absent; a configured list replaces it.
///
/// # Errors
/// The [`LeadsError`] when the file cannot be read or `orchestrate` is not valid.
pub(crate) fn orchestrate_leads(root: &Path) -> Result<Leads, LeadsError> {
    match read_text(root)? {
        Some(text) => leads_from_text(&text),
        None => Ok(Leads::default_leads()),
    }
}

/// The orchestrate leads from the config `text`.
///
/// # Errors
/// As [`orchestrate_leads`], for the setting.
pub(crate) fn leads_from_text(text: &str) -> Result<Leads, LeadsError> {
    let dto: LeadsDto = serde_json::from_str(text).map_err(LeadsError::Json)?;
    match dto.orchestrate.leads {
        Some(names) => Leads::try_from(names),
        None => Ok(Leads::default_leads()),
    }
}

#[cfg(test)]
mod tests;
