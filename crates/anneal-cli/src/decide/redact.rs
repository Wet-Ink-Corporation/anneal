//! Text made safe to send to Clef: the redaction rules every caller of `decide` shares, so
//! neither the hooks nor `grill classify` own them.
//!
//! - [`secret_assignment_prefix`] finds an environment-style or credential-named `NAME=value`
//!   (credential words in [`CREDENTIAL_WORDS`]); the guard's command redaction
//!   (`hook::guard::redact`) applies it to shell words.
//! - [`safe_prompt`] is prose as Clef sees it: whitespace collapsed, secrets replaced by
//!   [`REDACTED`], and cut to [`MAX_PROMPT_CHARS`] (`state::cut_chars`). Prose is not shell, so
//!   it gets the assignment rule word by word plus the rules prose needs: the word after
//!   `Bearer`, known token prefixes, AWS access key ids and long opaque words. It is a filter
//!   for the obvious cases, not a secret scanner. The prompt hooks send prompts through it, and
//!   `grill classify` a question and its goal.

use crate::decide::state::{MAX_STRING_CHARS, cut_chars};

/// What a redacted value is replaced with.
pub(crate) const REDACTED: &str = "<redacted>";

/// Words a credential name holds, compared in ASCII lower case.
pub(crate) const CREDENTIAL_WORDS: [&str; 10] = [
    "token",
    "password",
    "passwd",
    "secret",
    "apikey",
    "api-key",
    "api_key",
    "credential",
    "private-key",
    "private_key",
];

/// Most characters of prose sent to Clef; a longer text ends in `…`. The compact state's own
/// cap, as for the guard's command, so the state does not cut it again.
pub(crate) const MAX_PROMPT_CHARS: usize = MAX_STRING_CHARS;

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

/// `prompt` as it may be sent to Clef: see the module documentation.
pub(crate) fn safe_prompt(prompt: &str) -> String {
    let mut safe = String::new();
    let mut chars = 0_usize;
    let mut after_bearer = false;
    let mut words = prompt.split_whitespace();
    while let Some(word) = words.next() {
        let before = safe.len();
        if !safe.is_empty() {
            safe.push(' ');
        }
        // Punctuation around a word (`"TOKEN=x"`, `(KEY=y)`, `'ghp_…',`) hides neither an
        // assignment nor a token.
        let punctuation = |c: char| !(c.is_ascii_alphanumeric() || c == '_');
        let rest = word.trim_start_matches(punctuation);
        let lead = word
            .get(..word.len().saturating_sub(rest.len()))
            .unwrap_or("");
        let core = rest.trim_end_matches(punctuation);
        let trail = rest.get(core.len()..).unwrap_or("");
        if after_bearer || looks_like_token(core) {
            safe.push_str(lead);
            safe.push_str(REDACTED);
            safe.push_str(trail);
        } else if let Some(assignment) = secret_assignment_prefix(rest) {
            safe.push_str(lead);
            safe.push_str(assignment);
            safe.push_str(REDACTED);
            let value = rest.get(assignment.len()..).unwrap_or("");
            if !skip_quoted(value, &mut words) {
                // An unclosed quote: everything after it may be the secret.
                break;
            }
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

/// Consumes from `words` the rest of a quoted assignment `value`: when it opens a quote (`'` or
/// `"`) that its own word does not close, the words up to and including the one that does.
/// Returns `false` when no word closes it, so the rest of the prompt went with it.
fn skip_quoted<'p>(value: &str, words: &mut impl Iterator<Item = &'p str>) -> bool {
    let mut chars = value.chars();
    let Some(quote @ ('\'' | '"')) = chars.next() else {
        return true;
    };
    chars.as_str().contains(quote) || words.any(|word| word.contains(quote))
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

/// `NAME=`, `NAME+=`, `NAME[…]=` or `NAME[…]+=` at the start of `word`, through the `=`, when
/// `NAME` is an environment-style name or holds a credential word. The guard applies it to
/// shell words, [`safe_prompt`] to the words of prose.
pub(crate) fn secret_assignment_prefix(word: &str) -> Option<&str> {
    let name_len = word
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(word.len());
    let name = word.get(..name_len)?;
    if !(is_name(name) && (is_env_name(name) || is_credential(name))) {
        return None;
    }
    let mut end = name_len;
    if word.get(end..)?.starts_with('[') {
        let close = word.get(end..)?.find(']')?;
        end = end.saturating_add(close).saturating_add(1);
    }
    let rest = word.get(end..)?;
    let equals = if rest.starts_with("+=") {
        2
    } else if rest.starts_with('=') {
        1
    } else {
        return None;
    };
    word.get(..end.saturating_add(equals))
}

/// A shell identifier.
fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// An environment-style name: `[A-Z_][A-Z0-9_]*`.
fn is_env_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Whether a name holds a credential word, in any letter case.
pub(crate) fn is_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    CREDENTIAL_WORDS.iter().any(|word| name.contains(word))
}
