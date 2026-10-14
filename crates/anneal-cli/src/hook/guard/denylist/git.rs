//! `git push` forcing a protected branch, `git add -f` (or `git stage -f`), and global
//! settings that define an alias or run a command ([`settings`]).

mod settings;

pub(super) use settings::settings;

use super::wrappers::{Long, is_long, may_be_option, skip_one, skip_options, takes_value};
use crate::hook::guard::shell::{Shape, Word};
use crate::hook::guard::{Denial, Rule};

/// git's global options that take the next word as their value.
const GLOBAL_VALUES: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
    "--super-prefix",
];

/// `git push` options that take the next word as their value.
const PUSH_VALUES: &[&str] = &["--repo", "--receive-pack", "--exec", "--push-option"];

/// Checks `git <args>`.
pub(super) fn check(args: &[Word]) -> Option<Denial> {
    if let Some(setting) = settings(args).unseen {
        return Some(Denial::new(
            Rule::Unparseable,
            format!(
                "git {setting} may define an alias or a command the guard cannot see; run the \
                 command it stands for"
            ),
        ));
    }
    let [subcommand, rest @ ..] = skip_options(args, GLOBAL_VALUES) else {
        return None;
    };
    if !subcommand.is_literal() {
        return push(rest).or_else(|| add(rest));
    }
    match subcommand.text.as_str() {
        "push" => push(rest),
        "add" | "stage" => add(rest),
        _ => None,
    }
}

/// What `git push` was asked to do.
#[derive(Debug, Default)]
struct Push<'a> {
    /// The force flag given (`--force`, `-f`, `--force-with-lease`, `--mirror`), if any.
    force: Option<&'a str>,
    /// The flag that pushes every branch (`--all`, `--branches`, `--mirror`), if any.
    every_branch: Option<&'a str>,
    /// The repository and the refspecs.
    operands: Vec<&'a Word>,
}

impl<'a> Push<'a> {
    fn parse(args: &'a [Word]) -> Self {
        let mut push = Self::default();
        let mut rest = args;
        while let [first, tail @ ..] = rest {
            rest = tail;
            let text = first.text.as_str();
            if text == "--" {
                push.operands.extend(tail);
                break;
            }
            if !may_be_option(first) {
                push.operands.push(first);
                continue;
            }
            if text.starts_with("--") {
                push.long(text);
                if takes_value(text, PUSH_VALUES) {
                    rest = skip_one(rest);
                }
            } else if push.short(text) {
                rest = skip_one(rest);
            }
        }
        push
    }

    /// Reads a long option, possibly abbreviated (`--force-w`) or with a value.
    fn long(&mut self, flag: &'a str) {
        let Some(long) = Long::parse(flag) else {
            return;
        };
        if ["--force", "--force-with-lease", "--mirror"]
            .into_iter()
            .any(|option| long.is(option))
        {
            self.force = Some(flag);
        }
        if ["--all", "--branches", "--mirror"]
            .into_iter()
            .any(|option| long.is(option))
        {
            self.every_branch = Some(flag);
        }
    }

    /// Reads a short-option cluster such as `-fu`; true when its last option takes the next
    /// word as a value (`-o`).
    fn short(&mut self, cluster: &'a str) -> bool {
        for (at, flag) in cluster.char_indices().skip(1) {
            match flag {
                'f' => self.force = Some(cluster),
                // `-o <option>`: the rest of the cluster, or the next word, is its value.
                'o' => return at.saturating_add(1) == cluster.len(),
                _ => {}
            }
        }
        false
    }
}

/// Where a refspec pushes to.
#[derive(Debug, PartialEq, Eq)]
enum Destination<'a> {
    /// A protected branch (or a pattern that may match one).
    Protected(&'a str),
    /// No explicit destination: `HEAD`, a revision expression, a variable.
    Implicit,
    /// Another branch or ref.
    Other,
}

/// The destination of `refspec`.
fn destination(refspec: &Word) -> Destination<'_> {
    if refspec.shape == Shape::Dynamic {
        return Destination::Implicit;
    }
    let spec = refspec.text.strip_prefix('+').unwrap_or(&refspec.text);
    let target = spec.split_once(':').map_or(spec, |(_, target)| target);
    let implicit = target.is_empty()
        || target == "HEAD"
        || target == "@"
        || target.contains(['~', '^'])
        || target.contains("@{");
    if implicit {
        return Destination::Implicit;
    }
    let branch = target.strip_prefix("refs/heads/").unwrap_or(target);
    let protected = matches!(branch, "main" | "master")
        || branch.starts_with("release/")
        || branch.contains(['*', '?', '[']);
    if protected {
        Destination::Protected(branch)
    } else {
        Destination::Other
    }
}

fn push(args: &[Word]) -> Option<Denial> {
    let push = Push::parse(args);
    let refspecs = push
        .operands
        .split_first()
        .map_or(&[][..], |(_, refspecs)| refspecs);
    let plus = refspecs.iter().any(|refspec| refspec.text.starts_with('+'));
    if push.force.is_none() && !plus {
        return None;
    }
    let deny = |reason: String| Some(Denial::new(Rule::ForcePush, reason));
    if let Some(every) = push.every_branch {
        return deny(format!(
            "git push {every} force-pushes every branch, protected ones included"
        ));
    }
    if refspecs.is_empty() {
        return deny(
            "a force push with no refspec pushes the current branch, which may be protected; \
             name the destination, as in `git push --force origin HEAD:my-branch`"
                .to_owned(),
        );
    }
    let forced = refspecs
        .iter()
        .filter(|refspec| push.force.is_some() || refspec.text.starts_with('+'));
    for refspec in forced {
        match destination(refspec) {
            Destination::Protected(branch) => {
                return deny(format!(
                    "force push to protected branch {branch} (main, master and release/* are \
                     protected)"
                ));
            }
            Destination::Implicit => {
                return deny(format!(
                    "force push of {} has no explicit destination, which may be a protected \
                     branch; name it, as in HEAD:my-branch",
                    refspec.text
                ));
            }
            Destination::Other => {}
        }
    }
    None
}

fn add(args: &[Word]) -> Option<Denial> {
    let mut rest = args;
    while let [first, tail @ ..] = rest {
        rest = tail;
        let text = first.text.as_str();
        if text == "--" {
            break;
        }
        let force = may_be_option(first)
            && (is_long(text, "--force") || (!text.starts_with("--") && text.contains('f')));
        if force {
            return Some(Denial::new(
                Rule::ForceAdd,
                format!(
                    "git add {text} stages files .gitignore excludes; add them to the repository \
                     deliberately, or change .gitignore"
                ),
            ));
        }
    }
    None
}
