//! `gh pr merge`, read for the merge gate: which pull request it names, the repository it
//! names with `-R`/`--repo` or in a pull request URL, and the head commit `--match-head-commit`
//! pins.
//!
//! gh parses its options with cobra and pflag: options may come anywhere among the operands,
//! before `pr` too; long options are exact (no abbreviation); short options cluster (`-db 5` is
//! `-d -b 5`), and the first short option in a cluster that takes a value takes the rest of
//! the word, or the next word when it ends the cluster (`-R=o/x` drops the `=`).

use super::wrappers::may_be_option;
use crate::hook::guard::shell::Word;

/// Long options of `gh` and `gh pr merge` that take the next word as their value.
const VALUE_LONGS: &[&str] = &[
    "--author-email",
    "--body",
    "--body-file",
    "--match-head-commit",
    "--repo",
    "--subject",
];

/// Short options that take a value, by the long option they stand for.
const VALUE_SHORTS: &[(char, &str)] = &[
    ('A', "--author-email"),
    ('b', "--body"),
    ('F', "--body-file"),
    ('R', "--repo"),
    ('t', "--subject"),
];

/// The values `strconv.ParseBool` reads as true, which pflag uses for `--flag=<value>`.
const TRUE_VALUES: &[&str] = &["1", "t", "T", "TRUE", "true", "True"];

/// The pull request a `gh pr merge` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// By number, as `42`, `#42` or an `http(s)://<host>/<owner>/<repo>/pull/42` URL.
    Number(u64),
    /// None: gh merges the current branch's pull request.
    Missing,
    /// A branch name, any other word, or a word known only at run time.
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
    /// The `<host>/<owner>/<repo>` of a pull request URL, lower-cased, when the target is one.
    pub(crate) url_repo: Option<String>,
    /// `GH_REPO` or `GH_HOST`, when the command sets either anywhere.
    pub(crate) env: Option<String>,
    /// How the command changed the working directory before gh ran (`cd`, `env --chdir`, ...),
    /// so gh may read another repository's remotes.
    pub(crate) moved: Option<&'static str>,
}

/// What a pass over `gh`'s arguments found.
#[derive(Debug, Default)]
struct Parsed<'a> {
    repo: Option<&'a str>,
    match_head: Option<&'a str>,
    disable_auto: bool,
    operands: Vec<&'a Word>,
}

/// How an option word takes its value.
#[derive(Debug, Clone, Copy)]
enum Taken<'a> {
    /// It takes none (a boolean, or an option the gate does not read).
    Flag,
    /// Its value is attached: `--repo=o/x`, `-Ro/x`.
    Attached(&'a str),
    /// Its value is the next word.
    Next,
}

/// The `gh pr merge` that `gh <args>` runs, if it runs one. `--disable-auto` (or a true
/// `--disable-auto=<bool>`) turns auto-merge off and merges nothing. A group or subcommand word
/// known only at run time may be `pr merge`, so it counts as one whose pull request is unknown.
pub(crate) fn pr_merge(args: &[Word]) -> Option<PrMerge> {
    let parsed = parse(args);
    if parsed.disable_auto {
        return None;
    }
    let (target, url_repo) = target(&parsed.operands)?;
    Some(PrMerge {
        target,
        repo: parsed.repo.map(str::to_owned),
        match_head: parsed.match_head.map(str::to_owned),
        url_repo,
        env: None,
        moved: None,
    })
}

/// The target of `operands` when they run `pr merge`: they begin with `pr merge`, or with words
/// known only at run time that may be or hide it.
fn target(operands: &[&Word]) -> Option<(Target, Option<String>)> {
    let is = |word: &Word, text: &str| word.is_literal() && word.text == text;
    let may_be = |word: &Word, text: &str| !word.is_literal() || word.text == text;
    match operands {
        [group, command, rest @ ..] if is(group, "pr") && is(command, "merge") => Some(
            rest.first()
                .map_or((Target::Missing, None), |word| target_of(word)),
        ),
        [group, command, ..] if may_be(group, "pr") && may_be(command, "merge") => {
            Some((Target::Unnumbered, None))
        }
        [first, rest @ ..] if !first.is_literal() => {
            // A run-time word may expand to nothing or to options, so `pr merge` after it
            // still runs.
            let hidden = rest.windows(2).any(
                |pair| matches!(pair, [group, command] if is(group, "pr") && is(command, "merge")),
            );
            hidden.then_some((Target::Unnumbered, None))
        }
        _ => None,
    }
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
        let (name, taken) = read_option(&first.text);
        let value = match taken {
            Taken::Flag => None,
            Taken::Attached(value) => Some(value),
            Taken::Next => {
                let [value, after @ ..] = rest else {
                    break;
                };
                rest = after;
                Some(value.text.as_str())
            }
        };
        match (name, value) {
            ("--repo", Some(value)) => parsed.repo = Some(value),
            ("--match-head-commit", Some(value)) => parsed.match_head = Some(value),
            ("--disable-auto", None) => parsed.disable_auto = true,
            ("--disable-auto", Some(value)) => parsed.disable_auto = TRUE_VALUES.contains(&value),
            _ => {}
        }
    }
    parsed
}

/// The long option `text` stands for (or `text` itself), and how it takes its value.
fn read_option(text: &str) -> (&str, Taken<'_>) {
    if text.starts_with("--") {
        return match text.split_once('=') {
            Some((name, value)) => (name, Taken::Attached(value)),
            None if VALUE_LONGS.contains(&text) => (text, Taken::Next),
            None => (text, Taken::Flag),
        };
    }
    let Some(cluster) = text.strip_prefix('-') else {
        return (text, Taken::Flag);
    };
    let mut rest = cluster.chars();
    while let Some(short) = rest.next() {
        let Some(&(_, long)) = VALUE_SHORTS.iter().find(|(name, _)| *name == short) else {
            continue;
        };
        let after = rest.as_str();
        let after = after.strip_prefix('=').unwrap_or(after);
        return if after.is_empty() {
            (long, Taken::Next)
        } else {
            (long, Taken::Attached(after))
        };
    }
    (text, Taken::Flag)
}

/// The pull request `operand` names, and the repository of a URL naming it.
fn target_of(operand: &Word) -> (Target, Option<String>) {
    if !operand.is_literal() {
        return (Target::Unnumbered, None);
    }
    read_target(&operand.text)
}

/// The pull request the literal operand `text` names (`42`, `#42` or a pull request URL), and
/// the repository of a URL naming it.
pub(crate) fn read_target(text: &str) -> (Target, Option<String>) {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if let Ok(number) = digits.parse::<u64>() {
        return (Target::Number(number), None);
    }
    match pull_url(text) {
        Some((repo, number)) => (Target::Number(number), Some(repo)),
        None => (Target::Unnumbered, None),
    }
}

/// `http(s)://<host>/<owner>/<repo>/pull/<number>`, optionally followed by `/`, `?` or `#` and
/// anything: the repository as `<host>/<owner>/<repo>` (host without a port, all lower-cased)
/// and the number. Anything else, a URL with user information included, is `None`.
fn pull_url(text: &str) -> Option<(String, u64)> {
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))?;
    let mut parts = rest.splitn(5, '/');
    let (Some(host), Some(owner), Some(repo), Some("pull"), Some(tail)) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return None;
    };
    let host = host.split_once(':').map_or(host, |(name, _port)| name);
    if [host, owner, repo].iter().any(|part| part.is_empty()) || host.contains('@') {
        return None;
    }
    let end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    let (digits, after) = tail.split_at_checked(end)?;
    if !(after.is_empty() || after.starts_with(['/', '?', '#'])) {
        return None;
    }
    let number = digits.parse::<u64>().ok()?;
    Some((
        format!("{host}/{owner}/{repo}").to_ascii_lowercase(),
        number,
    ))
}
