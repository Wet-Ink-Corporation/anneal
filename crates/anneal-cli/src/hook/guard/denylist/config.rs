//! Bash writes to lint or hook config: redirections, `tee`, `truncate`, `sed -i`, `cp`, `mv`,
//! `ln`, `install`, `rm` and `chmod` aimed at a protected path.
//!
//! Paths are matched on their components, so `docs/hooks.md` and `myhooks/x` are not `hooks/`.
//! A component with an unquoted glob matches when it could match a protected name
//! (`.cl*/settings.json`).

use super::wrappers::{is_option, skip_one};
use crate::hook::guard::shell::{Shape, Word};
use crate::hook::guard::{Denial, Rule};

/// A path component's name.
#[derive(Debug, Clone, Copy)]
enum Name {
    Exact(&'static str),
    /// Starts with `prefix` and ends with `suffix`; a glob is tried against `samples`.
    Affix {
        prefix: &'static str,
        suffix: &'static str,
        samples: &'static [&'static str],
    },
}

impl Name {
    /// Whether `component` is this name; when `glob` and the component holds a wildcard,
    /// whether it could match it.
    fn matches(self, component: &str, glob: bool) -> bool {
        let wildcard = glob && component.contains(['*', '?', '[']);
        match self {
            Self::Exact(name) if wildcard => glob_match(component, name),
            Self::Exact(name) => component == name,
            Self::Affix { samples, .. } if wildcard => {
                samples.iter().any(|sample| glob_match(component, sample))
            }
            Self::Affix { prefix, suffix, .. } => component
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.ends_with(suffix)),
        }
    }
}

/// A protected path: a component, directly under `parent` when that is set.
#[derive(Debug)]
struct Protected {
    /// How a denial names it.
    label: &'static str,
    parent: Option<&'static str>,
    name: Name,
}

impl Protected {
    fn matches(&self, components: &[&str], glob: bool) -> bool {
        match self.parent {
            None => components.iter().any(|c| self.name.matches(c, glob)),
            Some(parent) => components.windows(2).any(|pair| {
                matches!(pair, [above, component]
                    if Name::Exact(parent).matches(above, glob) && self.name.matches(component, glob))
            }),
        }
    }
}

/// A file protected wherever it is.
const fn file(name: &'static str) -> Protected {
    Protected {
        label: name,
        parent: None,
        name: Name::Exact(name),
    }
}

/// The protected paths.
const PROTECTED: [Protected; 12] = [
    Protected {
        label: "hooks/",
        parent: None,
        name: Name::Exact("hooks"),
    },
    Protected {
        label: ".claude/settings*.json",
        parent: Some(".claude"),
        name: Name::Affix {
            prefix: "settings",
            suffix: ".json",
            samples: &["settings.json", "settings.local.json"],
        },
    },
    Protected {
        label: ".cargo/config*",
        parent: Some(".cargo"),
        name: Name::Affix {
            prefix: "config",
            suffix: "",
            samples: &["config", "config.toml"],
        },
    },
    Protected {
        label: ".github/workflows/",
        parent: Some(".github"),
        name: Name::Exact("workflows"),
    },
    Protected {
        label: ".eslintrc*",
        parent: None,
        name: Name::Affix {
            prefix: ".eslintrc",
            suffix: "",
            samples: &[
                ".eslintrc",
                ".eslintrc.json",
                ".eslintrc.js",
                ".eslintrc.cjs",
                ".eslintrc.yml",
            ],
        },
    },
    file("clippy.toml"),
    file(".clippy.toml"),
    file("rustfmt.toml"),
    file(".rustfmt.toml"),
    file("deny.toml"),
    file(".pre-commit-config.yaml"),
    file("lefthook.yml"),
];

/// Directories that hold protected config: removing, moving or re-permissioning one changes it.
const CONTAINERS: [(&str, &str); 3] = [
    (".claude", ".claude/"),
    (".github", ".github/"),
    (".cargo", ".cargo/"),
];

/// The components of a path, without empty and `.` ones.
fn components(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|component| !component.is_empty() && *component != ".")
        .collect()
}

/// The protected path `path` writes, if any.
fn protected(path: &str, shape: Shape) -> Option<&'static str> {
    let glob = shape != Shape::Literal;
    let components = components(path);
    PROTECTED
        .iter()
        .find(|protected| protected.matches(&components, glob))
        .map(|protected| protected.label)
}

/// The protected path removing (or re-permissioning) `path` changes, if any: a protected path
/// or a directory holding one.
fn contains_protected(path: &str, shape: Shape) -> Option<&'static str> {
    protected(path, shape).or_else(|| {
        let glob = shape != Shape::Literal;
        let last = *components(path).last()?;
        CONTAINERS
            .iter()
            .find(|(name, _)| Name::Exact(name).matches(last, glob))
            .map(|(_, label)| *label)
    })
}

fn deny(action: &str, label: &str) -> Denial {
    Denial::new(
        Rule::ConfigWrite,
        format!(
            "{action}: {label} is lint or hook config, which a Bash command may not change; \
             change it in an edit a human reviews"
        ),
    )
}

/// A redirection writing `target`.
pub(super) fn redirect(target: &Word) -> Option<Denial> {
    let label = protected(&target.text, target.shape)?;
    Some(deny(
        &format!("a redirection writes {}", target.text),
        label,
    ))
}

/// Checks `<program> <args>` for writes to protected config.
pub(super) fn check(program: &str, args: &[Word]) -> Option<Denial> {
    match program {
        "tee" | "truncate" => args.iter().find_map(|arg| {
            let label = protected(&arg.text, arg.shape)?;
            Some(deny(&format!("{program} writes {}", arg.text), label))
        }),
        "rm" | "unlink" | "rmdir" | "shred" | "chmod" | "chown" | "chgrp" => {
            args.iter().find_map(|arg| {
                let label = contains_protected(&arg.text, arg.shape)?;
                Some(deny(&format!("{program} changes {}", arg.text), label))
            })
        }
        "sed" => sed(args),
        "cp" | "install" | "ln" => copy(program, args, false),
        "mv" => copy(program, args, true),
        _ => None,
    }
}

/// The parts of a `sed` command line that matter here.
#[derive(Debug, Default)]
struct SedArgs<'a> {
    in_place: bool,
    /// The script came from `-e` or `-f`, so every operand is a file.
    script_given: bool,
    operands: Vec<&'a Word>,
}

impl<'a> SedArgs<'a> {
    fn parse(args: &'a [Word]) -> Self {
        let mut sed = Self::default();
        let mut rest = args;
        while let [first, tail @ ..] = rest {
            rest = tail;
            let text = first.text.as_str();
            if text == "--" {
                sed.operands.extend(tail);
                break;
            }
            if !is_option(first) {
                sed.operands.push(first);
            } else if text.starts_with("--in-place") {
                sed.in_place = true;
            } else if matches!(text, "--expression" | "--file") {
                sed.script_given = true;
                rest = skip_one(rest);
            } else if text.starts_with("--expression=") || text.starts_with("--file=") {
                sed.script_given = true;
            } else if !text.starts_with("--") && sed.cluster(text) {
                rest = skip_one(rest);
            }
        }
        sed
    }

    /// Reads a short-option cluster; true when its last option takes the next word.
    fn cluster(&mut self, cluster: &str) -> bool {
        for (at, flag) in cluster.char_indices().skip(1) {
            let last = at.saturating_add(1) == cluster.len();
            match flag {
                // `-i[SUFFIX]`: the rest of the cluster is the suffix.
                'i' => {
                    self.in_place = true;
                    return false;
                }
                'e' | 'f' => {
                    self.script_given = true;
                    return last;
                }
                'l' => return last,
                _ => {}
            }
        }
        false
    }
}

fn sed(args: &[Word]) -> Option<Denial> {
    let sed = SedArgs::parse(args);
    if !sed.in_place {
        return None;
    }
    let files = if sed.script_given {
        &sed.operands[..]
    } else {
        sed.operands
            .split_first()
            .map_or(&[][..], |(_, files)| files)
    };
    files.iter().find_map(|file| {
        let label = protected(&file.text, file.shape)?;
        Some(deny(&format!("sed -i edits {}", file.text), label))
    })
}

/// Short options of `cp`, `mv`, `ln` and `install` that take the next word as a value.
const COPY_VALUES: [char; 4] = ['S', 'm', 'o', 'g'];

/// The destination and sources of `cp`, `mv`, `ln` or `install`.
#[derive(Debug, Default)]
struct CopyArgs<'a> {
    /// `-t DIR` or `--target-directory=DIR`.
    target_dir: Option<(&'a str, Shape)>,
    operands: Vec<&'a Word>,
}

impl<'a> CopyArgs<'a> {
    fn parse(args: &'a [Word]) -> Self {
        let mut copy = Self::default();
        let mut rest = args;
        while let [first, tail @ ..] = rest {
            rest = tail;
            let text = first.text.as_str();
            if text == "--" {
                copy.operands.extend(tail);
                break;
            }
            if !is_option(first) {
                copy.operands.push(first);
            } else if text == "--target-directory" {
                copy.target_dir = tail.first().map(|dir| (dir.text.as_str(), dir.shape));
                rest = skip_one(rest);
            } else if let Some(dir) = text.strip_prefix("--target-directory=") {
                copy.target_dir = Some((dir, first.shape));
            } else if text == "--suffix" {
                rest = skip_one(rest);
            } else if text.starts_with("--") {
            } else if let Some((_, dir)) = text.split_once('t') {
                if dir.is_empty() {
                    copy.target_dir = tail.first().map(|dir| (dir.text.as_str(), dir.shape));
                    rest = skip_one(rest);
                } else {
                    copy.target_dir = Some((dir, first.shape));
                }
            } else if text.ends_with(COPY_VALUES) {
                rest = skip_one(rest);
            }
        }
        copy
    }
}

/// The last component of `path`.
fn basename(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

fn copy(program: &str, args: &[Word], moves: bool) -> Option<Denial> {
    let copy = CopyArgs::parse(args);
    // A source lands at `<dest>/<its name>` when the destination is a directory: always for
    // `-t`, and, since the guard cannot look, when the destination ends in `/` or names a
    // directory that holds config.
    let ((dest, dest_shape), sources, into_dir) = if let Some(dir) = copy.target_dir {
        (dir, &copy.operands[..], true)
    } else {
        let (last, sources) = copy.operands.split_last()?;
        let named_dir = last.text.ends_with(['/', '\\'])
            || components(&last.text)
                .last()
                .is_some_and(|name| CONTAINERS.iter().any(|(dir, _)| dir == name));
        ((last.text.as_str(), last.shape), sources, named_dir)
    };
    if sources.is_empty() {
        return None;
    }
    if let Some(label) = protected(dest, dest_shape) {
        return Some(deny(&format!("{program} writes {dest}"), label));
    }
    sources.iter().find_map(|source| {
        if moves && let Some(label) = contains_protected(&source.text, source.shape) {
            return Some(deny(&format!("{program} moves {}", source.text), label));
        }
        let joined = format!("{dest}/{}", basename(&source.text));
        let label = protected(&joined, dest_shape.max(source.shape)).filter(|_| into_dir)?;
        Some(deny(&format!("{program} writes {joined}"), label))
    })
}

/// Whether `text` matches the shell pattern `pattern` (`*`, `?`, and `[...]` as any one
/// character, which can only widen a match).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
            }
            Some('[') => {
                let close = pattern.iter().skip(p).position(|c| *c == ']');
                p = close.map_or(p + 1, |close| p + close + 1);
                t += 1;
            }
            Some(c) if *c == '?' || Some(c) == text.get(t) => {
                p += 1;
                t += 1;
            }
            _ => {
                let Some((star_p, star_t)) = star else {
                    return false;
                };
                p = star_p + 1;
                t = star_t + 1;
                star = Some((star_p, t));
            }
        }
    }
    pattern.iter().skip(p).all(|c| *c == '*')
}
