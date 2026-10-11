//! What the `Stop` and `SubagentStop` hooks write: Claude Code's top-level `decision` and
//! `reason`, the event's `additionalContext`, and a `systemMessage` for the one `ERROR` line
//! of a hook that failed open. Nothing at all when there is nothing to say.

use std::io::{self, Write};

use serde::Serialize;

/// The output of a `Stop` or `SubagentStop` hook.
#[derive(Debug, Serialize)]
pub(crate) struct StopOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    specific: Option<Specific>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
    #[serde(skip)]
    event: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Specific {
    hook_event_name: &'static str,
    additional_context: String,
}

impl StopOutput {
    /// An empty output for the hook event `event` (`Stop` or `SubagentStop`).
    pub(crate) const fn new(event: &'static str) -> Self {
        Self {
            decision: None,
            reason: None,
            specific: None,
            system_message: None,
            event,
        }
    }

    /// Blocks the stop, telling the model `reason`.
    #[must_use]
    pub(crate) fn block(mut self, reason: String) -> Self {
        self.decision = Some("block");
        self.reason = Some(reason);
        self
    }

    /// Adds `context` for the model without blocking.
    #[must_use]
    pub(crate) fn context(mut self, context: String) -> Self {
        self.specific = Some(Specific {
            hook_event_name: self.event,
            additional_context: context,
        });
        self
    }

    /// Shows the user `ERROR <reason>`, as one line.
    #[must_use]
    pub(crate) fn error(mut self, reason: &str) -> Self {
        self.system_message = Some(format!("ERROR {}", reason.replace(['\r', '\n'], " ")));
        self
    }

    /// Writes the output as one JSON line, or nothing when it is empty.
    ///
    /// # Errors
    /// The I/O error when `out` cannot be written.
    pub(crate) fn write<W: Write>(&self, mut out: W) -> io::Result<()> {
        if self.decision.is_none() && self.specific.is_none() && self.system_message.is_none() {
            return Ok(());
        }
        serde_json::to_writer(&mut out, self)?;
        writeln!(out)
    }
}
