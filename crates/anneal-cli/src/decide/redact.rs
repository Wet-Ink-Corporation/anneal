//! Text made safe to send to Clef: the redaction rules every caller of `decide` shares, so
//! neither the hooks nor `grill classify` own them.
//!
//! - [`secret_assignment_prefix`] finds an environment-style or credential-named `NAME=value`
//!   (credential words in [`CREDENTIAL_WORDS`]), quotes in the name stripped first
//!   (`"API_KEY"=x`); the guard's command redaction (`hook::guard::redact`) applies it to shell
//!   words.
//! - [`authorization_end`], [`scheme`], [`url_userinfo`] and [`tools`] find the credentials of
//!   an `Authorization:` header, the word after `Bearer` or `Basic`, a URL's `user:token@`, and
//!   the short options of known tools (`mysql -pX`, `curl -u user:pass`); the guard and the
//!   prose redaction both apply them.
//! - [`safe_prompt`] is prose as Clef sees it: whitespace collapsed, secrets replaced by
//!   [`REDACTED`], and cut to [`MAX_PROMPT_CHARS`] (`state::cut_chars`). Prose is not shell, so
//!   it gets the rules above word by word plus the rules prose needs: known token prefixes, AWS
//!   access key ids and long opaque words. It is a filter for the obvious cases, not a secret
//!   scanner. The prompt hooks send prompts through it, and `grill classify` a question and its
//!   goal.

pub(crate) mod tools;

use std::borrow::Cow;

use self::tools::{SecretOption, Tool};
use crate::decide::state::{MAX_STRING_CHARS, cut_chars};

/// What a redacted value is replaced with.
pub(crate) const REDACTED: &str = "<redacted>";

/// Words a credential name holds, compared in ASCII lower case.
pub(crate) const CREDENTIAL_WORDS: [&str; 13] = [
    "token",
    "password",
    "passwd",
    "pass",
    "secret",
    "apikey",
    "api-key",
    "api_key",
    "credential",
    "private-key",
    "private_key",
    "auth",
    "bearer",
];

/// Whole words that hold a credential word but name none: `author` (as in `--author=`). One
/// counts only when no letter follows it, so `authorization` is still a credential.
const NOT_CREDENTIALS: [&str; 2] = ["author", "authors"];

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

/// Words of prose redacted after an `Authorization:` header name: its scheme and credential.
const AUTHORIZATION_WORDS: usize = 2;

/// `prompt` as it may be sent to Clef: see the module documentation.
pub(crate) fn safe_prompt(prompt: &str) -> String {
    let mut prose = Prose::default();
    let mut chars = 0_usize;
    let mut words = prompt.split_whitespace();
    while let Some(word) = words.next() {
        let before = prose.safe.len();
        if !prose.safe.is_empty() {
            prose.safe.push(' ');
        }
        let went_on = prose.word(word, &mut words);
        let pushed = prose
            .safe
            .get(before..)
            .map_or(0, |piece| piece.chars().count());
        chars = chars.saturating_add(pushed);
        // Past the cap the rest would be cut anyway; an unclosed quote took the rest.
        if !went_on || chars > MAX_PROMPT_CHARS {
            break;
        }
    }
    cut_chars(&prose.safe, MAX_PROMPT_CHARS)
}

/// The prose made safe so far, and what the words before say about the next ones.
#[derive(Debug, Default)]
struct Prose {
    safe: String,
    /// How many of the next words are secrets (after `Bearer`, `Authorization:`, `curl -u`).
    pending: usize,
    /// The word before was `Basic`.
    after_basic: bool,
    /// The last known tool named, whose secret options are redacted from then on.
    tool: Option<Tool>,
}

impl Prose {
    /// Writes `word` made safe. Returns `false` when it opened a quote that no later word
    /// closes, so the rest of the prompt went with it.
    fn word<'p>(&mut self, word: &'p str, words: &mut impl Iterator<Item = &'p str>) -> bool {
        // Punctuation around a word (`"TOKEN=x"`, `(KEY=y)`, `'ghp_…',`) hides neither an
        // assignment nor a token.
        let rest = word.trim_start_matches(is_punctuation);
        let lead = word
            .get(..word.len().saturating_sub(rest.len()))
            .unwrap_or("");
        let core = rest.trim_end_matches(is_punctuation);
        let trail = rest.get(core.len()..).unwrap_or("");
        let hidden = self.pending > 0
            || (self.after_basic && basic_credential(core))
            || looks_like_token(core);
        self.pending = self.pending.saturating_sub(1);
        self.after_basic = false;
        let mut went_on = true;
        if hidden {
            self.safe.push_str(lead);
            self.safe.push_str(REDACTED);
            self.safe.push_str(trail);
        } else if let Some((before, prefix, value)) = assignment(word) {
            self.safe.push_str(before);
            self.safe.push_str(prefix);
            self.safe.push_str(REDACTED);
            went_on = skip_quoted(quote_after(quote_after(None, before), value), words);
        } else if let Some((name, value)) =
            authorization_end(word).and_then(|end| word.split_at_checked(end))
        {
            self.safe.push_str(name);
            if !value.is_empty() {
                self.safe.push_str(REDACTED);
            }
            self.pending = AUTHORIZATION_WORDS;
        } else if let Some(option) = self.tool.and_then(|tool| tool.secret_option(word)) {
            self.secret_option(word, option);
        } else {
            push_without_userinfo(word, &mut self.safe);
        }
        match scheme(core) {
            Some(Scheme::Bearer) => self.pending = self.pending.max(1),
            Some(Scheme::Basic) => self.after_basic = true,
            None => {}
        }
        if let Some(tool) = tools::known(core) {
            self.tool = Some(tool);
        }
        went_on
    }

    /// Writes a known tool's secret option: its value redacted in the word, or the next word.
    fn secret_option(&mut self, word: &str, option: SecretOption) {
        match option {
            SecretOption::Attached(kept) => {
                // `kept` ends within the ASCII option name, so the split exists.
                if let Some((option, _value)) = word.split_at_checked(kept) {
                    self.safe.push_str(option);
                }
                self.safe.push_str(REDACTED);
            }
            SecretOption::NextWord => {
                self.safe.push_str(word);
                self.pending = self.pending.max(1);
            }
        }
    }
}

/// Punctuation that may surround a word of prose.
fn is_punctuation(c: char) -> bool {
    !(c.is_ascii_alphanumeric() || c == '_')
}

/// The secret assignment in a word of prose, as its lead punctuation, the assignment through
/// the `=` and the value. Quotes may belong to the name (`"API_KEY"=x`), so the word is tried
/// with only other punctuation trimmed first, then with all of it (`"TOKEN=x"`).
fn assignment(word: &str) -> Option<(&str, &str, &str)> {
    let unquoted = word.trim_start_matches(|c| is_punctuation(c) && !matches!(c, '\'' | '"'));
    let rest = word.trim_start_matches(is_punctuation);
    [unquoted, rest].into_iter().find_map(|candidate| {
        let prefix = secret_assignment_prefix(candidate)?;
        let lead = word.get(..word.len().saturating_sub(candidate.len()))?;
        let value = candidate.get(prefix.len()..)?;
        Some((lead, prefix, value))
    })
}

/// The quote still open after `text`, given the one open before it: each `'` or `"` outside
/// a quote opens one, and the same character closes it.
fn quote_after(mut open: Option<char>, text: &str) -> Option<char> {
    for c in text.chars() {
        open = match open {
            Some(quote) if c == quote => None,
            None if matches!(c, '\'' | '"') => Some(c),
            other => other,
        };
    }
    open
}

/// Consumes from `words` the rest of a quoted value: while a quote is `open`, the words up to
/// and including the one that closes it. Returns `false` when no word closes it, so the rest
/// of the prompt went with it.
fn skip_quoted<'p>(mut open: Option<char>, words: &mut impl Iterator<Item = &'p str>) -> bool {
    while open.is_some() {
        let Some(word) = words.next() else {
            return false;
        };
        open = quote_after(open, word);
    }
    true
}

/// Pushes `word` with the userinfo of every URL in it redacted.
fn push_without_userinfo(word: &str, out: &mut String) {
    let mut rest = word;
    while let Some((start, end)) = url_userinfo(rest)
        && let (Some(head), Some(tail)) = (rest.get(..start), rest.get(end..))
    {
        out.push_str(head);
        out.push_str(REDACTED);
        rest = tail;
    }
    out.push_str(rest);
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

/// An HTTP authentication scheme whose next word is the credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scheme {
    /// `Bearer`: the next word is always redacted.
    Bearer,
    /// `Basic`: the next word is redacted when [`basic_credential`] says it may be one.
    Basic,
}

/// The scheme `word` names, in any letter case.
pub(crate) fn scheme(word: &str) -> Option<Scheme> {
    if word.eq_ignore_ascii_case("bearer") {
        Some(Scheme::Bearer)
    } else if word.eq_ignore_ascii_case("basic") {
        Some(Scheme::Basic)
    } else {
        None
    }
}

/// Whether the word after `Basic` may be a credential: anything but plain lower-case letters,
/// so `basic tests` stays readable and base64 (`dXNlcjpwYXNz`) does not.
pub(crate) fn basic_credential(word: &str) -> bool {
    !word.bytes().all(|b| b.is_ascii_lowercase())
}

/// Just past the first `authorization:` in `word`, in any letter case (`Authorization:`,
/// `Proxy-Authorization:`): what follows is the header's value.
pub(crate) fn authorization_end(word: &str) -> Option<usize> {
    const HEADER: &str = "authorization:";
    // ASCII lower-casing keeps every byte offset.
    let at = word.to_ascii_lowercase().find(HEADER)?;
    Some(at.saturating_add(HEADER.len()))
}

/// The byte range of the userinfo of the first URL in `word` that has one
/// (`https://user:token@host`): after `://`, up to the last `@` before the path, query or
/// fragment. `git@host:repo` has no `://`, so it is not a URL here.
pub(crate) fn url_userinfo(word: &str) -> Option<(usize, usize)> {
    word.match_indices("://").find_map(|(at, separator)| {
        let start = at.saturating_add(separator.len());
        let after = word.get(start..)?;
        let authority = after.get(..after.find(['/', '?', '#']).unwrap_or(after.len()))?;
        let userinfo = authority.rfind('@').filter(|&len| len > 0)?;
        Some((start, start.saturating_add(userinfo)))
    })
}

/// `NAME=`, `NAME+=`, `NAME[…]=` or `NAME[…]+=` at the start of `word`, through the `=`, when
/// `NAME` (its quotes stripped, as in `"API_KEY"=x`) is an environment-style name or holds a
/// credential word. The guard applies it to shell words, [`safe_prompt`] to the words of prose.
pub(crate) fn secret_assignment_prefix(word: &str) -> Option<&str> {
    let name_len = assignment_name_len(word)?;
    let quoted = word.get(..name_len)?;
    let name: Cow<'_, str> = if quoted.contains(['\'', '"']) {
        Cow::Owned(
            quoted
                .chars()
                .filter(|&c| !matches!(c, '\'' | '"'))
                .collect(),
        )
    } else {
        Cow::Borrowed(quoted)
    };
    if !(is_name(&name) && (is_env_name(&name) || is_credential(&name))) {
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

/// The length of the name at the start of `word`: name characters, some of them inside
/// balanced `'…'` or `"…"`. `None` when a quote opened there holds anything else (`"K=v"` is a
/// quoted word, not a quoted name) or is not closed.
fn assignment_name_len(word: &str) -> Option<usize> {
    let mut open = None;
    for (at, c) in word.char_indices() {
        match open {
            Some(quote) if c == quote => open = None,
            _ if c.is_ascii_alphanumeric() || c == '_' => {}
            None if matches!(c, '\'' | '"') => open = Some(c),
            None => return Some(at),
            Some(_) => return None,
        }
    }
    open.is_none().then_some(word.len())
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

/// Whether a name holds a credential word, in any letter case, other than as part of one of
/// [`NOT_CREDENTIALS`].
pub(crate) fn is_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    CREDENTIAL_WORDS.iter().any(|word| {
        name.match_indices(word).any(|(at, _)| {
            name.get(at..).is_none_or(|from| {
                !NOT_CREDENTIALS.iter().any(|benign| {
                    from.strip_prefix(benign)
                        .is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_alphabetic()))
                })
            })
        })
    })
}

#[cfg(test)]
mod tests;
