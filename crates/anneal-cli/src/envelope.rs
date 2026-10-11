//! The JSON envelope every command writes on stdout, and the findings it carries.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::command::CommandName;
use crate::exit_codes::Code;

/// One problem a command found. The `key` is stable across runs, so a baseline can tell old
/// findings from new ones (see [`crate::baseline`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// Stable identity: the same problem yields the same key on every run, e.g.
    /// `"lint/skill-name:plugin/skills/plan/SKILL.md"`.
    pub key: String,
    /// One-line human description.
    pub message: String,
}

impl Finding {
    /// A finding with a stable `key` and a one-line `message`.
    #[must_use]
    pub fn new(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            message: message.into(),
        }
    }
}

/// The result of a command, serialised as one JSON object on stdout:
/// `{"ok", "command", "data", "findings", "proves", "does_not_prove"}`.
///
/// `ok` is derived from the exit code, and [`Envelope::emit`] returns that same code, so the
/// JSON and the exit status cannot disagree.
#[derive(Debug, Serialize)]
pub struct Envelope<'a, D> {
    #[serde(skip)]
    code: Code,
    ok: bool,
    command: String,
    data: D,
    findings: &'a [Finding],
    proves: &'static str,
    does_not_prove: &'static str,
}

impl<'a, D: Serialize> Envelope<'a, D> {
    /// An envelope with no findings for `name` and its `subcommand`, if any, that will exit
    /// with `code`. The `command` field is built from both (`"dag check"`), and the proves /
    /// does-not-prove statements come from `name`, so they cannot disagree. `data` should
    /// serialise to a JSON object; use `serde_json::Map::new()` for `{}`.
    #[must_use]
    pub fn new(name: CommandName, subcommand: Option<&str>, code: Code, data: D) -> Self {
        let claim = name.claim();
        let command = match subcommand {
            Some(sub) => format!("{name} {sub}"),
            None => name.as_str().to_owned(),
        };
        Self {
            code,
            ok: code.is_success(),
            command,
            data,
            findings: &[],
            proves: claim.proves,
            does_not_prove: claim.does_not_prove,
        }
    }

    /// The same envelope carrying `findings`.
    #[must_use]
    pub fn with_findings(self, findings: &'a [Finding]) -> Self {
        Self { findings, ..self }
    }

    /// Writes the envelope as one line of JSON and returns the exit code it was built with.
    ///
    /// # Errors
    /// Returns the I/O error when `out` cannot be written or `data` fails to serialise.
    pub fn emit<W: Write>(&self, mut out: W) -> io::Result<Code> {
        serde_json::to_writer(&mut out, self)?;
        out.write_all(b"\n")?;
        Ok(self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::{Envelope, Finding};
    use crate::command::CommandName;
    use crate::exit_codes::{DAG_CYCLE, OK};
    use serde_json::{Value, json};

    fn emitted(envelope: &Envelope<'_, Value>) -> (u8, Value) {
        let mut out = Vec::new();
        let code = envelope.emit(&mut out).expect("a Vec accepts every write");
        assert_eq!(out.last(), Some(&b'\n'), "one line, newline-terminated");
        let value = serde_json::from_slice(&out).expect("emit writes valid JSON");
        (code.value(), value)
    }

    #[test]
    fn envelope_serialises_the_six_documented_fields() {
        let findings = [Finding::new("dag/cycle:a-b", "a depends on b depends on a")];
        let envelope = Envelope::new(
            CommandName::Dag,
            Some("check"),
            DAG_CYCLE,
            json!({"tasks": 2}),
        )
        .with_findings(&findings);

        let (code, value) = emitted(&envelope);

        assert_eq!(code, 1, "emit returns the envelope's code");
        assert_eq!(
            value,
            json!({
                "ok": false,
                "command": "dag check",
                "data": {"tasks": 2},
                "findings": [{"key": "dag/cycle:a-b", "message": "a depends on b depends on a"}],
                "proves": CommandName::Dag.claim().proves,
                "does_not_prove": CommandName::Dag.claim().does_not_prove,
            }),
            "the wire shape is the documented one"
        );
    }

    #[test]
    fn ok_is_true_only_for_exit_code_zero() {
        let green = Envelope::new(CommandName::Lint, None, OK, json!({}));
        let red = Envelope::new(CommandName::Dag, Some("check"), DAG_CYCLE, json!({}));

        assert_eq!(emitted(&green).1["ok"], json!(true), "0 is ok");
        assert_eq!(emitted(&red).1["ok"], json!(false), "1 is not ok");
    }

    #[test]
    fn command_is_the_name_alone_without_a_subcommand() {
        let envelope = Envelope::new(CommandName::FloorGuard, None, OK, json!({}));
        assert_eq!(
            emitted(&envelope).1["command"],
            json!("floor-guard"),
            "the name as typed"
        );
    }
}
