//! Text made safe to send to Clef: the redaction rules every caller of `decide` shares, so
//! neither the hooks nor `grill classify` own them.
//!
//! - [`rules`] reads one word: [`find_secret`] (assignments, credential-named keys and
//!   options, the options of the known tools in [`tools`]), [`Scheme`] (`Bearer`, `Basic`),
//!   [`url_userinfo`] and [`looks_like_token`]. The guard's command redaction
//!   (`hook::guard::redact`) applies them to shell words with its quoting rules.
//! - [`safe_prompt`] is prose as Clef sees it: whitespace collapsed, secrets replaced by
//!   [`REDACTED`], and cut to [`MAX_PROMPT_CHARS`] (`state::cut_chars`). It applies the same
//!   word rules, carrying what one word says about the next (a credential option's value, a
//!   header's value, a quote a secret opens) across words, past a lone `\` (a line
//!   continuation), which is no word, and reading a word that ends in one without it; an
//!   assignment word owes the next word nothing. It is a filter for the obvious cases, not a
//!   secret scanner. The prompt hooks send prompts through it, and `grill classify` a
//!   question and its goal.
//! - [`judge`] reads one shell word's text as bash passes it on (quotes removed, line
//!   continuations dropped) with the same rules: whether it still holds a secret, and what it
//!   owes the word after it. The guard judges each word with it.

pub(crate) mod judge;
mod prose;
pub(crate) mod rules;
pub(crate) mod tools;

pub(crate) use self::prose::safe_prompt;
pub(crate) use self::rules::{
    Scheme, Secret, Then, basic_credential, find_secret, looks_like_token, scheme,
    split_punctuation, url_userinfo,
};
use crate::decide::state::MAX_STRING_CHARS;

/// What a redacted value is replaced with.
pub(crate) const REDACTED: &str = "<redacted>";

/// Most characters of prose sent to Clef; a longer text ends in `…`. The compact state's own
/// cap, as for the guard's command, so the state does not cut it again.
pub(crate) const MAX_PROMPT_CHARS: usize = MAX_STRING_CHARS;

#[cfg(test)]
mod tests;
