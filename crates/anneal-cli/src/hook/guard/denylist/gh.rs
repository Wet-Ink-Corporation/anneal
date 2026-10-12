//! `gh pr merge`, read for the merge gate: which pull request it names, the repository it
//! names with `-R`/`--repo`, and the head commit `--match-head-commit` pins. gh (cobra) takes
//! options anywhere among the operands and parses long options exactly, without
//! abbreviation.

use super::wrappers::may_be_option;
use crate::hook::guard::shell::Word;

/// `gh pr` and `gh pr merge` options that take the next word as their value.
const WITH_VALUE: &[&str] = &[
    "-A",
    "--author-email",
    "-b",
    "--body",
    "-F",
    "--body-file",
    "--match-head-commit",
    "-R",
    "--repo",
    "-t",
    "--subject",
];

/// The pull request a `gh pr merge` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// By number, as `42`, `#42` or a `.../pull/42` URL.
    Number(u64),
    /// None: gh merges the current branch's pull request.
    Missing,
    /// A branch name, or a word known only at run time.
    Unnumbered,
}

/// A `gh pr merge` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrMerge {
    /// The pull request.
    pub(crate) target: Target,
    /// `-R`/`--repo`, if given.
    pub(crate) repo: Option<String>,
    /// `--match-head-commit`, if given.
    pub(crate) match_head: Option<String>,
}

/// What a pass over `gh pr`'s arguments found.
#[derive(Debug, Default)]
struct Parsed<'a> {
    repo: Option<&'a str>,
    match_head: Option<&'a str>,
    disable_auto: bool,
    operands: Vec<&'a Word>,
}

/// The `gh pr merge` that `gh <args>` runs, if it runs one. `--disable-auto` turns auto-merge
/// off and merges nothing. A group or subcommand word known only at run time may be
/// `pr merge`, so it counts as one whose pull request is unknown.
pub(crate) fn pr_merge(args: &[Word]) -> Option<PrMerge> {
    let [group, rest @ ..] = args else {
        return None;
    };
    if group.is_literal() && group.text != "pr" {
        return None;
    }
    let parsed = parse(rest);
    let [command, operands @ ..] = parsed.operands.as_slice() else {
        return None;
    };
    if (command.is_literal() && command.text != "merge") || parsed.disable_auto {
        return None;
    }
    let target = if group.is_literal() && command.is_literal() {
        operands
            .first()
            .map_or(Target::Missing, |operand| target_of(operand))
    } else {
        Target::Unnumbered
    };
    Some(PrMerge {
        target,
        repo: parsed.repo.map(str::to_owned),
        match_head: parsed.match_head.map(str::to_owned),
    })
}

/// One pass over `words`: the options the gate reads, wherever they are, and the operands in
/// order (everything after `--` is an operand).
fn parse(words: &[Word]) -> Parsed<'_> {
    let mut parsed = Parsed::default();
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        rest = tail;
        if first.text == "--" {
            parsed.operands.extend(tail);
            break;
        }
        if !may_be_option(first) {
            parsed.operands.push(first);
            continue;
        }
        let (name, attached) = split_option(&first.text);
        let value = match attached {
            Some(value) => Some(value),
            None if WITH_VALUE.contains(&name) => {
                let [value, after @ ..] = rest else {
                    break;
                };
                rest = after;
                Some(value.text.as_str())
            }
            None => None,
        };
        match (name, value) {
            ("-R" | "--repo", Some(value)) => parsed.repo = Some(value),
            ("--match-head-commit", Some(value)) => parsed.match_head = Some(value),
            ("--disable-auto", _) => parsed.disable_auto = true,
            _ => {}
        }
    }
    parsed
}

/// An option's name and its attached value: `--repo=o/r` and `-Ro/r` (a short option that
/// takes a value) give one; anything else is the name alone.
fn split_option(text: &str) -> (&str, Option<&str>) {
    if text.starts_with("--") {
        return match text.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (text, None),
        };
    }
    let short = WITH_VALUE
        .iter()
        .copied()
        .filter(|option| option.len() == 2)
        .find(|option| text.len() > 2 && text.starts_with(option));
    match short.and_then(|name| Some((name, text.get(name.len()..)?))) {
        Some((name, value)) => (name, Some(value)),
        None => (text, None),
    }
}

/// The pull request `operand` names.
fn target_of(operand: &Word) -> Target {
    if !operand.is_literal() {
        return Target::Unnumbered;
    }
    let text = operand.text.as_str();
    let digits = text.strip_prefix('#').unwrap_or(text);
    if let Ok(number) = digits.parse::<u64>() {
        return Target::Number(number);
    }
    text.split_once("/pull/")
        .and_then(|(_, after)| {
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok()
        })
        .map_or(Target::Unnumbered, Target::Number)
}
