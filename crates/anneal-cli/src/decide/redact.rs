//! Text made safe to send to Clef: the redaction rules every caller of `decide` shares, so
//! neither the hooks nor `grill classify` own them.
//!
//! - [`rules`] reads one word: `rules::find_secret` (assignments, credential-named keys and
//!   options, the options of the known tools in [`tools`]), `rules::Scheme` (`Bearer`,
//!   `Basic`), `rules::url_userinfo` and `rules::looks_like_token`.
//! - [`safe_prompt`] is prose as Clef sees it: whitespace collapsed, secrets replaced by
//!   [`REDACTED`], and cut to [`MAX_PROMPT_CHARS`] (`state::cut_chars`). It applies the word
//!   rules, carrying what one word says about the next (a credential option's value, a
//!   header's value, a quote a secret opens) across words. It is a filter for the obvious
//!   cases, not a secret scanner. The prompt hooks send prompts through it, and
//!   `grill classify` a question and its goal.
//!
//! The guard's Bash command does not go through these rules: `hook::guard::redact` sends Clef
//! an allowlist skeleton of it, sharing only [`REDACTED`].

mod prose;
mod rules;
mod tools;

pub(crate) use self::prose::safe_prompt;
use crate::decide::state::MAX_STRING_CHARS;

/// What a redacted value is replaced with.
pub(crate) const REDACTED: &str = "<redacted>";

/// Most characters of prose sent to Clef; a longer text ends in `…`. The compact state's own
/// cap, as for the guard's command, so the state does not cut it again.
pub(crate) const MAX_PROMPT_CHARS: usize = MAX_STRING_CHARS;

#[cfg(test)]
mod tests;
