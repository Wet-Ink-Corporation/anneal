//! Reading Claude Code transcripts (JSONL: one JSON object per line), bounded (H-13).
//!
//! An assistant message is written as one line per content block, each carrying the message's
//! `id` and a `usage` snapshot, so a message's text is joined across its lines and its usage is
//! counted once. Lines that do not parse (a line cut by a tail read, a format change) are
//! passed over.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use anneal_ledger::Timestamp;
use anneal_ledger::event::TokenUsage;
use serde::Deserialize;
use serde_json::Value;

/// What marks a tool call in a transcript line. Inside a JSON string the quotes are escaped,
/// so text that merely mentions it does not match.
const TOOL_USE: &[u8] = br#""type":"tool_use""#;

/// Whether a transcript grew by a tool call since an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Activity {
    /// A tool call was written, or the transcript was rewritten or grew past the cap, so work
    /// is assumed to have happened.
    ToolCall,
    /// No tool call was written.
    Quiet,
}

/// What a subagent's transcript says about its run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Usage {
    /// From the first to the last timestamp, in milliseconds.
    pub(crate) duration_ms: Option<u64>,
    /// The tokens of each assistant message, counted once, summed.
    pub(crate) tokens: Option<TokenUsage>,
    /// The text of its last assistant message.
    pub(crate) last_text: Option<String>,
}

/// One transcript line, as far as these readers need it.
#[derive(Debug, Deserialize)]
struct Line {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    usage: Option<RawUsage>,
}

/// The API's usage object, in tokens; absent counts are zero.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct RawUsage {
    #[serde(default, rename = "input_tokens")]
    input: u64,
    #[serde(default, rename = "output_tokens")]
    output: u64,
    #[serde(default, rename = "cache_read_input_tokens")]
    cache_read: u64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    cache_creation: u64,
}

/// The lines of `bytes` that parse.
fn lines(bytes: &[u8]) -> impl Iterator<Item = Line> + '_ {
    // A line that does not parse is a line cut by a tail read or a format this reader does not
    // know; it carries nothing these readers can use, so it is passed over, not reported.
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
}

/// The text of the last assistant message in `bytes` (a transcript or its tail): the `text`
/// blocks of every line with that message's id, joined by newlines.
pub(crate) fn last_assistant_text(bytes: &[u8]) -> Option<String> {
    let assistant: Vec<Message> = lines(bytes)
        .filter(|line| line.kind.as_deref() == Some("assistant"))
        .filter_map(|line| line.message)
        .collect();
    let last_id = assistant.last()?.id.as_deref();
    let start = assistant
        .iter()
        .rposition(|message| message.id.as_deref() != last_id)
        .map_or(0, |at| at.saturating_add(1));
    let texts: Vec<&str> = assistant
        .get(start..)
        .unwrap_or_default()
        .iter()
        .flat_map(|message| texts(message.content.as_ref()))
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

/// The `text` of each text block of `content` (a string counts as one).
fn texts(content: Option<&Value>) -> Vec<&str> {
    match content {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect(),
        Some(_) | None => Vec::new(),
    }
}

/// `path`'s length, refusing anything but a regular file (a directory cannot be opened on
/// Windows, a pipe would block).
fn file_len(path: &Path) -> io::Result<u64> {
    let metadata = std::fs::metadata(path)?;
    if metadata.is_file() {
        Ok(metadata.len())
    } else {
        Err(io::Error::other(format!(
            "{} is not a regular file",
            path.display()
        )))
    }
}

/// The bytes of `path` from `start`, at most `max` of them.
fn read_from(path: &Path, start: u64, max: u64) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The last `max` bytes of the transcript at `path` (all of it when shorter).
///
/// # Errors
/// The I/O error when it cannot be read, or is not a regular file.
pub(crate) fn read_tail(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let len = file_len(path)?;
    read_from(path, len.saturating_sub(max), max)
}

/// The transcript's length now: where the next [`activity_since`] starts.
///
/// # Errors
/// The I/O error when it cannot be read, or is not a regular file.
pub(crate) fn transcript_len(path: &Path) -> io::Result<u64> {
    file_len(path)
}

/// Whether the transcript at `path` gained a tool call after byte `offset`. Shorter than
/// `offset` (rewritten, as by compaction) or grown by more than `max` bytes counts as
/// [`Activity::ToolCall`] without reading.
///
/// # Errors
/// The I/O error when it cannot be read, or is not a regular file.
pub(crate) fn activity_since(path: &Path, offset: u64, max: u64) -> io::Result<Activity> {
    let len = file_len(path)?;
    let Some(grown) = len.checked_sub(offset) else {
        return Ok(Activity::ToolCall);
    };
    if grown > max {
        return Ok(Activity::ToolCall);
    }
    let bytes = read_from(path, offset, grown)?;
    if bytes
        .windows(TOOL_USE.len())
        .any(|window| window == TOOL_USE)
    {
        Ok(Activity::ToolCall)
    } else {
        Ok(Activity::Quiet)
    }
}

/// The run figures of the transcript at `path`, or `None` when it is over `max` bytes (it is
/// then not read).
///
/// # Errors
/// The I/O error when it cannot be read, or is not a regular file.
pub(crate) fn usage(path: &Path, max: u64) -> io::Result<Option<Usage>> {
    if file_len(path)? > max {
        return Ok(None);
    }
    // One byte past the cap: a transcript that grew since its length was read is cut, and a
    // cut last line does not parse, so it is passed over.
    let bytes = read_from(path, 0, max.saturating_add(1))?;
    let mut per_message: BTreeMap<String, RawUsage> = BTreeMap::new();
    let mut unnamed = Vec::new();
    let (mut first, mut last): (Option<i64>, Option<i64>) = (None, None);
    for line in lines(&bytes) {
        // A line without a readable timestamp does not move the span.
        if let Some(at) = line
            .timestamp
            .and_then(|ts| Timestamp::try_from(ts).ok())
            .and_then(|ts| ts.unix_millis())
        {
            first = Some(first.map_or(at, |seen| seen.min(at)));
            last = Some(last.map_or(at, |seen| seen.max(at)));
        }
        let Some(Message {
            id,
            usage: Some(raw),
            ..
        }) = line.message
        else {
            continue;
        };
        match id {
            // Each line of a message repeats its usage; the latest is the message's.
            Some(id) => {
                per_message.insert(id, raw);
            }
            None => unnamed.push(raw),
        }
    }
    let counted: Vec<RawUsage> = per_message.into_values().chain(unnamed).collect();
    let duration_ms = first
        .zip(last)
        .and_then(|(first, last)| u64::try_from(last.saturating_sub(first)).ok());
    Ok(Some(Usage {
        duration_ms,
        tokens: (!counted.is_empty()).then(|| sum(&counted)),
        last_text: last_assistant_text(&bytes),
    }))
}

/// The field-wise sum of `usages`, saturating.
fn sum(usages: &[RawUsage]) -> TokenUsage {
    usages
        .iter()
        .fold(TokenUsage::default(), |total, raw| TokenUsage {
            input: total.input.saturating_add(raw.input),
            output: total.output.saturating_add(raw.output),
            cache_read: total.cache_read.saturating_add(raw.cache_read),
            cache_creation: total.cache_creation.saturating_add(raw.cache_creation),
        })
}

#[cfg(test)]
mod tests;
