//! The one shape of merge command the gate judges: a Bash command that is nothing but
//! `gh pr merge <number> --match-head-commit <commit> --repo <host>/<owner>/<repo>`, with a few
//! flags that cannot change what is merged or where.
//!
//! This is an allowlist read from the raw text, not from the guard's lexer: every word is
//! plain (letters, digits and `._/:@%+,=-`), so bash neither expands, splits nor drops it, and
//! the command has no separator, pipe, redirection, assignment, wrapper, subshell, heredoc or
//! comment. Only the value of `--subject` or `--body` may be quoted: in single quotes, or in
//! double quotes holding no `$`, backtick, `\` or `!`, so the value is one word and runs
//! nothing. Anything else holding a merge is denied (see `merge_gate`), however the guard
//! would read it.

use crate::hook::guard::{PrMerge, Target, read_target};

/// The command to run, as a denial names it.
pub(super) const FORM: &str =
    "gh pr merge <number> --match-head-commit <full commit> --repo <host>/<owner>/<repo>";

/// Flags that take no value and change neither the pull request, its head nor the repository.
const FLAGS: [&str; 5] = [
    "--auto",
    "--delete-branch",
    "--merge",
    "--rebase",
    "--squash",
];

/// Options whose value is text for the merge commit, which may be quoted.
const TEXTS: [&str; 2] = ["--body", "--subject"];

/// The most words an accepted command has (H-13): `gh pr merge`, the operand, three value
/// options and five flags, with room for repeats.
const MAX_WORDS: usize = 32;

/// One word of the command: its text, without quotes, and whether it was quoted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token<'a> {
    text: &'a str,
    quoted: bool,
}

/// How an option word takes its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value<'a> {
    /// The next word (`--repo x`, `-R x`).
    Next,
    /// Attached to the option (`--repo=x`, `-Rx`, `-R=x`).
    Attached(&'a str),
}

/// What an accepted command's arguments name, the last of each option winning as in pflag.
#[derive(Debug, Default)]
struct Named<'a> {
    operand: Option<&'a str>,
    repo: Option<&'a str>,
    pin: Option<&'a str>,
}

/// The merge `command` runs when it has the accepted shape; `None` for any other command.
pub(super) fn read(command: &str) -> Option<PrMerge> {
    let tokens = words(command)?;
    let [gh, pr, merge, args @ ..] = tokens.as_slice() else {
        return None;
    };
    let named = [gh, pr, merge]
        .iter()
        .zip(["gh", "pr", "merge"])
        .all(|(token, text)| !token.quoted && token.text == text)
        .then(|| arguments(args))??;
    let (target, url_repo) = named.operand.map_or((Target::Missing, None), read_target);
    Some(PrMerge {
        target,
        repo: named.repo.map(str::to_owned),
        match_head: named.pin.map(str::to_owned),
        url_repo,
        env: None,
        moved: None,
    })
}

/// Splits `command` at blanks into plain and quoted words; `None` when any part of it is
/// neither.
fn words(command: &str) -> Option<Vec<Token<'_>>> {
    let mut tokens = Vec::new();
    let mut rest = command.trim_matches([' ', '\t', '\n']);
    while !rest.is_empty() {
        let (token, after) = match rest.strip_prefix(['\'', '"']) {
            Some(body) => quoted(rest, body)?,
            None => plain(rest)?,
        };
        if !(after.is_empty() || after.starts_with([' ', '\t'])) || tokens.len() >= MAX_WORDS {
            return None;
        }
        tokens.push(token);
        rest = after.trim_start_matches([' ', '\t']);
    }
    Some(tokens)
}

/// The quoted word at the start of `text` (`body` is `text` after its opening quote), and
/// the text after its closing quote.
fn quoted<'a>(text: &'a str, body: &'a str) -> Option<(Token<'a>, &'a str)> {
    let quote = text.chars().next()?;
    let (inside, after) = body.split_once(quote)?;
    let runs_something = quote == '"' && inside.contains(['$', '`', '\\', '!']);
    (!runs_something).then_some((
        Token {
            text: inside,
            quoted: true,
        },
        after,
    ))
}

/// The plain word at the start of `text`, and the text after it.
fn plain(text: &str) -> Option<(Token<'_>, &str)> {
    let end = text.find([' ', '\t']).unwrap_or(text.len());
    let (word, after) = text.split_at_checked(end)?;
    let is_plain = |c: char| c.is_ascii_alphanumeric() || "._/:@%+,=-".contains(c);
    word.chars().all(is_plain).then_some((
        Token {
            text: word,
            quoted: false,
        },
        after,
    ))
}

/// What `args`, the words after `gh pr merge`, name; `None` when one is not accepted.
fn arguments<'a>(args: &[Token<'a>]) -> Option<Named<'a>> {
    let mut named = Named::default();
    let mut rest = args;
    while let [first, tail @ ..] = rest {
        rest = tail;
        if first.quoted {
            return None;
        }
        let text = first.text;
        if FLAGS.contains(&text) {
            continue;
        }
        if TEXTS.contains(&text) {
            let [_value, after @ ..] = rest else {
                return None;
            };
            rest = after;
            continue;
        }
        let option = value(text, "--repo", Some("-R"))
            .map(|how| (&mut named.repo, how))
            .or_else(|| value(text, "--match-head-commit", None).map(|how| (&mut named.pin, how)));
        let Some((slot, how)) = option else {
            if text.starts_with('-') || named.operand.is_some() {
                return None;
            }
            named.operand = Some(text);
            continue;
        };
        *slot = Some(match how {
            Value::Attached(attached) => attached,
            Value::Next => {
                let [next, after @ ..] = rest else {
                    return None;
                };
                rest = after;
                (!next.quoted).then_some(next.text)?
            }
        });
    }
    Some(named)
}

/// How `text` gives the option `long` (or its short form `short`) a value, if it is that
/// option.
fn value<'a>(text: &'a str, long: &str, short: Option<&str>) -> Option<Value<'a>> {
    if text == long || Some(text) == short {
        return Some(Value::Next);
    }
    if let Some(attached) = text
        .strip_prefix(long)
        .and_then(|rest| rest.strip_prefix('='))
    {
        return Some(Value::Attached(attached));
    }
    let attached = short.and_then(|short| text.strip_prefix(short))?;
    Some(Value::Attached(
        attached.strip_prefix('=').unwrap_or(attached),
    ))
}

#[cfg(test)]
mod tests;
