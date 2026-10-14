//! The word rules the prose redaction ([`super::safe_prompt`]) applies. They are structural and
//! conservative, so a new tool or header needs no new row: what names a credential
//! ([`is_credential`]) is enough.
//!
//! [`find_secret`] reads one word, in this order:
//! 1. an assignment at its start whose name, quotes stripped, is environment-style or names a
//!    credential (`TOKEN=x`, `"API_KEY"=x`, `db_pass=x`);
//! 2. a credential-named key anywhere in it, before `=` or `:` with optional quotes between
//!    (`--from-literal=password=x`, `-Dpassword=x`, `/:_authToken=x`, `{"password":"x"}`,
//!    `X-Api-Key:`, `"Authorization":`); a key with nothing after its `:`, or an
//!    `Authorization` one, is a header whose value runs on ([`Then::Header`]); a header-shaped
//!    one with text after its `:` (`Cookie:a=1;`, `X-Api-Key:k`) hides only that text, and
//!    after a `;` that ends it the cookie pairs that follow ([`Then::AttachedHeader`]);
//! 3. a secret option of a known tool ([`super::tools`]);
//! 4. an option, one dash or two, whose name names a credential (`--password x`,
//!    `-storepass x`): the next word is its value, unless the name says the value is read
//!    elsewhere (`--password-stdin`, `--password-file`, `--token-env`);
//! 5. a whole word that is a dotted or underscored configuration key naming a credential
//!    (`aws_secret_access_key x`, `user.password x`): the next word is its value, with the
//!    same exceptions as rule 4.

use std::borrow::Cow;

use super::tools::{SecretOption, Tool};

/// Words a credential name holds, compared in ASCII lower case.
pub(crate) const CREDENTIAL_WORDS: [&str; 14] = [
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
    "cookie",
];

/// Whole words that hold a credential word but name none: `author` (as in `--author=`). One
/// counts only when no letter follows it, so `authorization` is still a credential.
const NOT_CREDENTIALS: [&str; 2] = ["author", "authors"];

/// Endings of a credential-named option or key whose value is not on the command line.
const READ_ELSEWHERE: [&str; 3] = ["-stdin", "-file", "-env"];

/// Prefixes of well-known credential formats: `sk-` API keys, GitHub, Slack and GitLab tokens,
/// Vault service and batch tokens, and JSON web tokens (`eyJ`). Vault's legacy `s.` tokens are
/// too plain a prefix; they are found as `vault login`'s argument ([`super::tools`]).
const TOKEN_PREFIXES: [&str; 15] = [
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
    "hvs.",
    "hvb.",
    "eyJ",
];

/// Shortest word a prefix rule redacts, so a bare `sk-` is kept.
const MIN_PREFIXED_TOKEN: usize = 8;

/// Shortest word redacted for its shape alone: letters and digits mixed, no path separators.
const MIN_OPAQUE_TOKEN: usize = 32;

/// What the words after a secret are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Then {
    /// Not secret.
    Nothing,
    /// The next word is the secret (`--password x`, `curl -u x`).
    NextWord,
    /// A header's value runs on: through its quote or the next two words.
    Header,
    /// A header-shaped key's value is attached to its word (`Cookie:a=1`, `X-Api-Key:k`): that
    /// word alone is the secret, unless a `;` ends it, after which the next `name=value` words
    /// are the header's other cookie pairs ([`is_pair`]).
    AttachedHeader,
}

/// A secret found in a word: write [`Secret::shown`], then the redaction mark when
/// [`Secret::redacts`], then treat the next words as [`Secret::then`] says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Secret<'w> {
    /// What stays readable: the word up to its secret (quotes removed for a known tool's option).
    pub(crate) shown: Cow<'w, str>,
    /// Whether the rest of the word is secret; `false` when the value is the next word.
    pub(crate) redacts: bool,
    pub(crate) then: Then,
}

impl<'w> Secret<'w> {
    /// `word` kept through `kept` bytes, the rest of it secret.
    fn after(word: &'w str, kept: usize, then: Then) -> Option<Self> {
        Some(Self {
            shown: Cow::Borrowed(word.get(..kept)?),
            redacts: kept < word.len(),
            then,
        })
    }

    /// `word` kept whole, its value the next word.
    fn before_next(word: &'w str) -> Self {
        Self {
            shown: Cow::Borrowed(word),
            redacts: false,
            then: Then::NextWord,
        }
    }
}

/// The first secret in `word` by the rules in the module documentation; `tool` is the known
/// tool named before it, if any.
pub(crate) fn find_secret(word: &str, tool: Option<Tool>) -> Option<Secret<'_>> {
    if let Some(prefix) = secret_assignment_prefix(word) {
        return Secret::after(word, prefix.len(), Then::Nothing);
    }
    credential_key(word)
        .or_else(|| tool.and_then(|tool| tool_option(tool, word)))
        .or_else(|| credential_option(word))
        .or_else(|| config_key(word))
}

/// Characters of a key or option name.
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Rule 2: a credential-named key followed, after any quotes, by `=` or a single `:`.
fn credential_key(word: &str) -> Option<Secret<'_>> {
    let mut start = None;
    // A sentinel past the end closes the last name.
    let ends = word.char_indices().chain([(word.len(), ' ')]);
    for (at, c) in ends {
        if is_key_char(c) {
            start.get_or_insert(at);
            continue;
        }
        let Some(from) = start.take() else {
            continue;
        };
        let name = word.get(from..at)?;
        let after = word.get(at..)?;
        let rest = after.trim_start_matches(['"', '\'']);
        let separator = at.saturating_add(after.len().saturating_sub(rest.len()));
        let kept = separator.saturating_add(1);
        let then = if rest.starts_with('=') {
            Then::Nothing
        } else if rest.starts_with(':') && !rest.starts_with("::") {
            let lower = name.to_ascii_lowercase();
            if kept >= word.len() || lower.contains("authorization") {
                Then::Header
            } else if is_header_name(&lower) {
                Then::AttachedHeader
            } else {
                Then::Nothing
            }
        } else {
            continue;
        };
        if is_credential(name) {
            return Secret::after(word, kept, then);
        }
    }
    None
}

/// Whether a credential key's name, `lower` in ASCII lower case, has the shape of an HTTP
/// header, so a value attached to its `:` may go on in cookie pairs after a `;` (`Cookie:a=1;`):
/// `cookie`, or a name with a `-` inside it that ends in a credential word (`X-Api-Key`,
/// `PRIVATE-TOKEN`, `Set-Cookie`). Such a name may be no header (`db-password:/p`,
/// `fix-auth:main`), so its attached value hides only its own word ([`Then::AttachedHeader`]).
fn is_header_name(lower: &str) -> bool {
    lower == "cookie"
        || (lower.trim_start_matches('-').contains('-')
            && CREDENTIAL_WORDS.iter().any(|word| lower.ends_with(word)))
}

/// Rule 3: a known tool's secret option, read with shell quotes removed (`'-pX'`).
fn tool_option(tool: Tool, word: &str) -> Option<Secret<'_>> {
    let plain = dequote(word);
    match tool.secret_option(&plain)? {
        SecretOption::Attached(kept) => Some(Secret {
            shown: cut(plain, kept)?,
            redacts: true,
            then: Then::Nothing,
        }),
        SecretOption::NextWord => Some(Secret::before_next(word)),
    }
}

/// `plain` cut after `kept` bytes, borrowing when it can.
fn cut(plain: Cow<'_, str>, kept: usize) -> Option<Cow<'_, str>> {
    match plain {
        Cow::Borrowed(text) => text.get(..kept).map(Cow::Borrowed),
        Cow::Owned(mut text) => {
            if !text.is_char_boundary(kept) {
                return None;
            }
            text.truncate(kept);
            Some(Cow::Owned(text))
        }
    }
}

/// Rule 4: `-name` or `--name` naming a credential, with its value in the next word.
fn credential_option(word: &str) -> Option<Secret<'_>> {
    let name = word.strip_prefix("--").or_else(|| word.strip_prefix('-'))?;
    let option = name.chars().all(is_key_char) && names_value_here(name);
    option.then(|| Secret::before_next(word))
}

/// Rule 5: a configuration key naming a credential, alone in its word and set from the next
/// one. A `.` or `_` marks it as a key, so a plain word such as `password` stays prose.
fn config_key(word: &str) -> Option<Secret<'_>> {
    let key = !word.starts_with('-')
        && word.contains(['.', '_'])
        && word.chars().all(|c| is_key_char(c) || c == '.')
        && names_value_here(word);
    key.then(|| Secret::before_next(word))
}

/// Whether `name` names a credential whose value is given on the command line, not read from
/// standard input, a file or the environment ([`READ_ELSEWHERE`]).
fn names_value_here(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && is_credential(name)
        && !READ_ELSEWHERE.iter().any(|ending| lower.ends_with(ending))
}

/// `word` without shell quotes and backslashes, borrowed when it has none.
pub(crate) fn dequote(word: &str) -> Cow<'_, str> {
    const QUOTES: [char; 3] = ['\'', '"', '\\'];
    if word.contains(QUOTES) {
        Cow::Owned(word.chars().filter(|c| !QUOTES.contains(c)).collect())
    } else {
        Cow::Borrowed(word)
    }
}

/// `NAME=`, `NAME+=`, `NAME[…]=` or `NAME[…]+=` at the start of `word`, through the `=`, when
/// `NAME` (its quotes stripped, as in `"API_KEY"=x`) is an environment-style name or holds a
/// credential word.
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

/// Characters that substitute or escape in a shell word rather than only quote: a word holding
/// one may run a command, so a cookie pair ([`is_pair`]) holds none.
const SUBSTITUTING: [char; 4] = ['`', '$', '\\', '('];

/// A `name=value` word (`b=2`), as the pairs of a cookie are, read through the punctuation
/// around it: only quotes before a name of letters, digits, `_` and `-` (so not an option), a
/// value that may be quoted (`sid="S"`) or end in a quote that closes an outer one (`sid=S'`),
/// and no substitution or escape anywhere, so a hidden pair never hides a command. Its value,
/// quotes included, holds no blank or `;` and closes any quote it opens (`c='rm -rf ~'` is no
/// pair, read whole or split into words), so it cannot hold a command either.
pub(crate) fn is_pair(word: &str) -> bool {
    let (lead, core, trail) = split_punctuation(word);
    !word.contains(SUBSTITUTING)
        && lead.chars().all(|c| matches!(c, '\'' | '"'))
        && core.split_once('=').is_some_and(|(name, value)| {
            // A quote in the trailing punctuation alone closes an outer one (`sid=S'`).
            let open = quote_after(None, value);
            !name.is_empty()
                && name.chars().all(is_key_char)
                && !value.contains(|c: char| c.is_whitespace() || c == ';')
                && (open.is_none() || quote_after(open, trail).is_none())
        })
}

/// The quote still open after `text`, given the one open before it: each `'` or `"` outside
/// a quote opens one, and the same character closes it.
pub(crate) fn quote_after(mut open: Option<char>, text: &str) -> Option<char> {
    for c in text.chars() {
        open = match open {
            Some(quote) if c == quote => None,
            None if matches!(c, '\'' | '"') => Some(c),
            other => other,
        };
    }
    open
}

/// `word` split into its leading punctuation, its core and its trailing punctuation, so that
/// `("ghp_…"),` is read as `ghp_…`.
pub(crate) fn split_punctuation(word: &str) -> (&str, &str, &str) {
    let core_start = word
        .len()
        .saturating_sub(word.trim_start_matches(is_punctuation).len());
    // Both cuts come from trimming whole chars, so they are char boundaries.
    let (lead, rest) = word.split_at_checked(core_start).unwrap_or(("", word));
    let core_len = rest.trim_end_matches(is_punctuation).len();
    let (core, trail) = rest.split_at_checked(core_len).unwrap_or((rest, ""));
    (lead, core, trail)
}

/// Punctuation that may surround a word.
pub(crate) fn is_punctuation(c: char) -> bool {
    !(c.is_ascii_alphanumeric() || c == '_')
}

/// A known credential prefix, an AWS access key id, or a long opaque word.
pub(crate) fn looks_like_token(word: &str) -> bool {
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

/// The byte range of the userinfo of the first URL in `word` that has one
/// (`https://user:token@host`): after `://`, up to the last `@` before the path or query (a
/// `#` there may be part of a password). `git@host:repo` has no `://`, so it is not a URL here.
pub(crate) fn url_userinfo(word: &str) -> Option<(usize, usize)> {
    word.match_indices("://").find_map(|(at, separator)| {
        let start = at.saturating_add(separator.len());
        let after = word.get(start..)?;
        let authority = after.get(..after.find('/').unwrap_or(after.len()))?;
        let userinfo = without_query(authority).rfind('@').filter(|&len| len > 0)?;
        Some((start, start.saturating_add(userinfo)))
    })
}

/// The part of a URL's authority (`authority`, up to its path) that may hold userinfo: up to a
/// `?` that starts the query, but through one inside a password, which a `user:` before it and
/// an `@` after it show (`user:p?X@host`). `host?to=a@b` has no `:` before its `?`.
fn without_query(authority: &str) -> &str {
    match authority.split_once('?') {
        Some((before, after)) if !(before.contains(':') && after.contains('@')) => before,
        Some(_) | None => authority,
    }
}
