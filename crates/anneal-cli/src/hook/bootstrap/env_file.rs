//! Puts the plugin's `libexec` on PATH for later Bash calls.
//!
//! Claude Code gives a `SessionStart` hook `CLAUDE_ENV_FILE`, a shell script it sources before
//! each later Bash command. The bootstrap appends one line there that puts
//! `$CLAUDE_PLUGIN_ROOT/libexec`, the directory of the `anneal` launcher, at the front of PATH,
//! so the skills' bare `anneal ...` commands run. The launcher never runs itself through PATH,
//! so the prepended directory cannot make it recurse.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::error_chain::reason;

/// The variable naming the script Claude Code sources before each Bash command.
const ENV_FILE_VAR: &str = "CLAUDE_ENV_FILE";

/// The variable naming the plugin's root directory.
const PLUGIN_ROOT_VAR: &str = "CLAUDE_PLUGIN_ROOT";

/// How much of the env file is read to find an earlier copy of the line; past it the line is
/// appended without the check (H-13).
const MAX_ENV_FILE_BYTES: u64 = 64 * 1024;

/// Why the line was not written.
#[derive(Debug, thiserror::Error)]
enum ExportError {
    /// The plugin root is not UTF-8, so it cannot be written into the script as text.
    #[error("the plugin directory is not UTF-8")]
    NotUtf8,
    /// The plugin root holds a control character or PATH's separator.
    #[error("the plugin directory holds {0:?}, which a PATH line cannot carry")]
    Unsafe(char),
    /// The env file could not be opened.
    #[error("opening {ENV_FILE_VAR}")]
    Open(#[source] io::Error),
    /// The env file could not be read.
    #[error("reading {ENV_FILE_VAR}")]
    Read(#[source] io::Error),
    /// The env file could not be appended to.
    #[error("writing {ENV_FILE_VAR}")]
    Write(#[source] io::Error),
}

/// Appends the PATH line to `$CLAUDE_ENV_FILE` when it and `$CLAUDE_PLUGIN_ROOT` are both set
/// and the file does not already hold it; the note to show when that failed. With either
/// variable unset or empty, nothing is written and there is no note.
pub(super) fn export_libexec(lookup: impl Fn(&str) -> Option<OsString>) -> Option<String> {
    let file = lookup(ENV_FILE_VAR).filter(|value| !value.is_empty())?;
    let root = lookup(PLUGIN_ROOT_VAR).filter(|value| !value.is_empty())?;
    // Joined as text with `/`, not `Path::join`, so the line is the same on every platform.
    let written = root
        .to_str()
        .ok_or(ExportError::NotUtf8)
        .map(|root| format!("{}/libexec", root.trim_end_matches(['/', '\\'])))
        .and_then(|dir| export_line(&dir))
        .and_then(|line| append_once(Path::new(&file), &line));
    written.err().map(|error| {
        format!(
            "anneal did not put its libexec on PATH for Bash, so run it as the plugin's \
             libexec/anneal: {}",
            reason(&error)
        )
    })
}

/// `export PATH='<dir>':"$PATH"` and a newline, `dir` single-quoted for sh with each `'`
/// written `'\''`; refused when `dir` holds a control character or `:`, PATH's separator.
fn export_line(dir: &str) -> Result<String, ExportError> {
    if let Some(unsafe_char) = dir.chars().find(|c| c.is_control() || *c == ':') {
        return Err(ExportError::Unsafe(unsafe_char));
    }
    let quoted = dir.replace('\'', r"'\''");
    Ok(format!("export PATH='{quoted}':\"$PATH\"\n"))
}

/// Appends `line` (ending in a newline) to `file`, creating it, unless a line of its first
/// [`MAX_ENV_FILE_BYTES`] already equals it; starts a new line first when the file read does
/// not end in one, or was too big to read whole.
fn append_once(file: &Path, line: &str) -> Result<(), ExportError> {
    let mut handle = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(file)
        .map_err(ExportError::Open)?;
    let mut held = Vec::new();
    Read::by_ref(&mut handle)
        .take(MAX_ENV_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut held)
        .map_err(ExportError::Read)?;
    let whole = u64::try_from(held.len()).is_ok_and(|len| len <= MAX_ENV_FILE_BYTES);
    let wanted = line.trim_end_matches('\n').as_bytes();
    if whole
        && held
            .split(|byte| *byte == b'\n')
            .any(|held_line| held_line.strip_suffix(b"\r").unwrap_or(held_line) == wanted)
    {
        return Ok(());
    }
    let mut text = String::with_capacity(line.len().saturating_add(1));
    if !whole || held.last().is_some_and(|byte| *byte != b'\n') {
        text.push('\n');
    }
    text.push_str(line);
    handle
        .write_all(text.as_bytes())
        .map_err(ExportError::Write)
}

#[cfg(test)]
mod tests;
