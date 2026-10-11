//! What the `UserPromptSubmit` hooks share: their input, the prompt made safe to send, their
//! time budget, and their output (one line of context and an `ERROR` line as `systemMessage`,
//! each only when there is one).
//!
//! The prompt Clef sees is [`safe_prompt`]: whitespace collapsed, secrets replaced by
//! `<redacted>`, and cut to [`MAX_PROMPT_CHARS`] (`decide::state::cut_chars`). A prompt is
//! prose, not shell, so it gets the guard's assignment rule word by word
//! (`guard::redact::secret_assignment_prefix`: an environment-style or credential-named
//! `NAME=value`, the credential words shared through `guard::redact::CREDENTIAL_WORDS`) plus
//! the rules prose needs: the word after `Bearer`, known token prefixes, AWS access key ids and
//! long opaque words. It is a filter for the obvious cases, not a secret scanner; the log keeps
//! only the state's hash, never the prompt.
//!
//! A hook's time is its [`Budget`]: [`HOOK_BUDGET`] from its start, measured on the monotonic
//! clock, so Clef is asked only while its whole timeout still fits.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::decide::client;
use crate::decide::clock::Clock;
use crate::decide::state::{MAX_STRING_CHARS, cut_chars};
use crate::hook::guard::redact::{REDACTED, secret_assignment_prefix};
use crate::hook::{StdinError, read_stdin};

/// Most bytes of hook input read; a prompt rides in it (H-13).
pub(crate) const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024;

/// Most characters of the prompt sent to Clef; a longer one ends in `…`. The compact state's own
/// cap, as for the guard's command, so the state does not cut it again.
pub(crate) const MAX_PROMPT_CHARS: usize = MAX_STRING_CHARS;

/// How long a prompt hook may take: its `hooks.json` timeout is 5 s, and the rest is left for
/// starting the process and writing the output.
pub(crate) const HOOK_BUDGET: Duration = Duration::from_millis(4_500);

/// Prefixes of well-known credential formats: `sk-` API keys, GitHub, Slack and GitLab tokens,
/// and JSON web tokens (`eyJ`).
const TOKEN_PREFIXES: [&str; 13] = [
    "sk-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "xoxa-",
    "xoxb-",
    "xoxp-",
    "xoxs-",
    "glpat-",
    "eyJ",
];

/// Shortest word a prefix rule redacts, so a bare `sk-` in prose is kept.
const MIN_PREFIXED_TOKEN: usize = 8;

/// Shortest word redacted for its shape alone: letters and digits mixed, no path separators.
const MIN_OPAQUE_TOKEN: usize = 32;

/// The fields of Claude Code's `UserPromptSubmit` input the hooks use; the rest are ignored.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Input {
    /// The session's id.
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    /// The session's transcript.
    #[serde(default)]
    pub(crate) transcript_path: Option<PathBuf>,
    /// The session's working directory.
    #[serde(default)]
    pub(crate) cwd: Option<PathBuf>,
    /// The prompt as submitted.
    #[serde(default)]
    pub(crate) prompt: Option<String>,
}

/// Why the hook input could not be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InputError {
    /// stdin could not be read, or was too large.
    #[error(transparent)]
    Stdin(#[from] StdinError),
    /// stdin was not `UserPromptSubmit` JSON.
    #[error("the input is not UserPromptSubmit JSON")]
    Json(#[source] serde_json::Error),
}

/// Reads and parses at most [`MAX_INPUT_BYTES`] of `stdin`.
///
/// # Errors
/// [`InputError`] when it cannot be read, is too large or does not parse.
pub(crate) fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES)?;
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// The prompt as it may be sent to Clef: see the module documentation.
pub(crate) fn safe_prompt(prompt: &str) -> String {
    let mut safe = String::new();
    let mut chars = 0_usize;
    let mut after_bearer = false;
    for word in prompt.split_whitespace() {
        let before = safe.len();
        if !safe.is_empty() {
            safe.push(' ');
        }
        // Punctuation before a word (`"TOKEN=x"`, `(KEY=y)`) does not hide an assignment.
        let start = word.len().saturating_sub(
            word.trim_start_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .len(),
        );
        let (lead, rest) = word.split_at_checked(start).unwrap_or(("", word));
        if after_bearer || looks_like_token(word) {
            safe.push_str(REDACTED);
        } else if let Some(assignment) = secret_assignment_prefix(rest) {
            safe.push_str(lead);
            safe.push_str(assignment);
            safe.push_str(REDACTED);
        } else {
            safe.push_str(word);
        }
        after_bearer = word.eq_ignore_ascii_case("bearer");
        let pushed = safe.get(before..).map_or(0, |piece| piece.chars().count());
        chars = chars.saturating_add(pushed);
        // Past the cap the rest would be cut anyway.
        if chars > MAX_PROMPT_CHARS {
            break;
        }
    }
    cut_chars(&safe, MAX_PROMPT_CHARS)
}

/// A prompt hook's time: [`HOOK_BUDGET`] from the hook's start, on the monotonic clock.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    start: Duration,
}

impl Budget {
    /// A budget starting now on `clock`.
    pub(crate) fn start(clock: &impl Clock) -> Self {
        Self {
            start: clock.monotonic(),
        }
    }

    /// What is left of the budget on `clock`, zero once spent.
    pub(crate) fn left(&self, clock: &impl Clock) -> Duration {
        let spent = clock.monotonic().saturating_sub(self.start);
        HOOK_BUDGET.saturating_sub(spent)
    }

    /// Whether Clef's whole timeout (`client::TIMEOUT`) still fits on `clock`.
    ///
    /// # Errors
    /// The note to write when it does not: Clef is then not asked.
    pub(crate) fn fits_clef(&self, clock: &impl Clock) -> Result<(), String> {
        let left = self.left(clock);
        if left >= client::TIMEOUT {
            Ok(())
        } else {
            Err(format!(
                "Clef was not asked: {} ms of the hook's {} ms left, under Clef's {} ms timeout",
                left.as_millis(),
                HOOK_BUDGET.as_millis(),
                client::TIMEOUT.as_millis()
            ))
        }
    }
}

/// A known credential prefix, an AWS access key id, or a long opaque word.
fn looks_like_token(word: &str) -> bool {
    let prefixed = word.len() >= MIN_PREFIXED_TOKEN
        && TOKEN_PREFIXES.iter().any(|prefix| word.starts_with(prefix));
    let aws = word.len() == 20
        && word.starts_with("AKIA")
        && word
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    let opaque = word.len() >= MIN_OPAQUE_TOKEN
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-+=".contains(&b))
        && word.bytes().any(|b| b.is_ascii_digit())
        && word.bytes().any(|b| b.is_ascii_alphabetic());
    prefixed || aws || opaque
}

/// A `UserPromptSubmit` hook's output: at most one line of context and one `ERROR` line.
#[derive(Debug, Default)]
pub(crate) struct Output {
    /// The line of context for the model, if any.
    pub(crate) context: Option<String>,
    /// The `ERROR <reason>` line for the human, if something failed.
    pub(crate) error: Option<String>,
}

#[derive(Debug, Serialize)]
struct OutputJson<'a> {
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<&'a str>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    specific: Option<SpecificJson<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecificJson<'a> {
    hook_event_name: &'static str,
    additional_context: &'a str,
}

impl Output {
    /// Writes the output as one JSON object and a newline, line breaks folded into spaces, or
    /// nothing when there is nothing to say. Pass `&mut out` to keep the writer.
    ///
    /// # Errors
    /// The I/O error when `out` cannot be written.
    pub(crate) fn write<W: Write>(&self, mut out: W) -> io::Result<()> {
        if self.context.is_none() && self.error.is_none() {
            return Ok(());
        }
        let one_line = |text: &str| text.replace(['\r', '\n'], " ");
        let context = self.context.as_deref().map(one_line);
        let error = self.error.as_deref().map(one_line);
        let json = OutputJson {
            system_message: error.as_deref(),
            specific: context.as_deref().map(|line| SpecificJson {
                hook_event_name: "UserPromptSubmit",
                additional_context: line,
            }),
        };
        serde_json::to_writer(&mut out, &json)?;
        writeln!(out)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Budget, HOOK_BUDGET, MAX_PROMPT_CHARS, Output, safe_prompt};
    use crate::hook::clef_fake::Stepping;

    #[test]
    fn whitespace_is_collapsed_and_the_ends_trimmed() {
        assert_eq!(
            safe_prompt("  build\n\tthe   next task \n"),
            "build the next task"
        );
    }

    #[test]
    fn environment_assignment_values_are_redacted() {
        assert_eq!(
            safe_prompt("run with API_KEY=abc123 and export DB_PASS='p w' then go"),
            "run with API_KEY=<redacted> and export DB_PASS=<redacted> w' then go"
        );
        assert_eq!(
            safe_prompt("set x=1 in the loop"),
            "set x=1 in the loop",
            "lower-case names are code, not environment"
        );
    }

    #[test]
    fn credential_named_assignments_are_redacted_in_any_case_and_behind_punctuation() {
        assert_eq!(
            safe_prompt("try api_token=abc, \"Password=hunter2\" or (K[0]=v) and PATH+=/x"),
            "try api_token=<redacted> \"Password=<redacted> or (K[0]=<redacted> and \
             PATH+=<redacted>"
        );
    }

    #[test]
    fn obvious_tokens_are_redacted() {
        let prompt = "use sk-proj-abcdefghijklmnop or ghp_0123456789abcdefghij \
                      github_pat_11AB_cd or xoxb-1-2 or AKIAIOSFODNN7EXAMPLE or glpat-xyz \
                      with Bearer eyJhbGciOi.payload and \
                      a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8 but keep build-the-next-task";

        assert_eq!(
            safe_prompt(prompt),
            "use <redacted> or <redacted> <redacted> or <redacted> or <redacted> or \
             <redacted> with Bearer <redacted> and <redacted> but keep build-the-next-task"
        );
    }

    #[test]
    fn the_prompt_is_cut_to_its_character_cap_with_an_ellipsis() {
        let long = "é".repeat(MAX_PROMPT_CHARS + 5);

        let safe = safe_prompt(&long);

        assert_eq!(
            safe.chars().count(),
            MAX_PROMPT_CHARS + 1,
            "the cap plus `…`"
        );
        assert!(safe.ends_with('…'), "a cut is marked");
        assert!(
            safe.chars().take(MAX_PROMPT_CHARS).all(|c| c == 'é'),
            "no broken character"
        );
    }

    #[test]
    fn a_prompt_redacted_shorter_is_kept_up_to_the_cap() {
        let token = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8";
        let words = vec![token; MAX_PROMPT_CHARS];

        let safe = safe_prompt(&words.join(" "));

        assert_eq!(safe.chars().count(), MAX_PROMPT_CHARS + 1, "{safe}");
        assert!(!safe.contains(token), "every token redacted");
    }

    #[test]
    fn the_budget_counts_from_the_start_and_skips_clef_when_its_timeout_no_longer_fits() {
        let clock = Stepping::by_ms(1_000);

        let budget = Budget::start(&clock);

        assert!(budget.fits_clef(&clock).is_ok(), "1 s spent, 3.5 s left");
        assert!(budget.fits_clef(&clock).is_ok(), "2 s spent, 2.5 s left");
        let note = budget.fits_clef(&clock).expect_err("3 s spent, 1.5 s left");
        assert_eq!(
            note,
            "Clef was not asked: 1500 ms of the hook's 4500 ms left, under Clef's 2000 ms timeout"
        );
        assert_eq!(HOOK_BUDGET, Duration::from_millis(4_500));
        assert_eq!(budget.left(&clock), Duration::from_millis(500), "4 s spent");
        assert_eq!(budget.left(&clock), Duration::ZERO, "spent, never negative");
    }

    #[test]
    fn nothing_to_say_writes_nothing() {
        let mut out = Vec::new();

        Output::default()
            .write(&mut out)
            .expect("a Vec accepts every write");

        assert!(out.is_empty(), "silent on pass");
    }

    #[test]
    fn a_context_line_and_an_error_are_one_json_object() {
        let mut out = Vec::new();
        let output = Output {
            context: Some("anneal route: one\nline".to_owned()),
            error: Some("ERROR route: the log".to_owned()),
        };

        output.write(&mut out).expect("a Vec accepts every write");

        let text = String::from_utf8(out).expect("UTF-8");
        assert_eq!(text.lines().count(), 1, "{text}");
        let json: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(
            json,
            serde_json::json!({
                "systemMessage": "ERROR route: the log",
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptSubmit",
                    "additionalContext": "anneal route: one line"
                }
            })
        );
    }
}
