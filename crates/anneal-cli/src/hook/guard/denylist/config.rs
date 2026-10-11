//! Bash writes to lint or hook config: redirections, `tee`, `truncate`, `sed -i`, `cp`, `mv`,
//! `ln`, `install`, `rm` and `chmod` aimed at a protected path.
//!
//! Paths are matched on their components ([`paths`]), so `docs/hooks.md` and `myhooks/x` are
//! not `hooks/`. A component with an unquoted glob matches when it could match a protected name
//! (`.cl*/settings.json`). `..` is resolved lexically; a relative path that climbs above where
//! it starts may land under any parent, so `../settings.json` counts as `.claude/settings.json`.
//! A relative path after a `cd` is also read from the directory the `cd` named, and from
//! anywhere after a `cd` the guard cannot follow.

mod paths;

pub(super) use paths::Cwd;
use paths::{CONTAINERS, contains_protected, last_component, protected};

use super::wrappers::{Long, is_option, skip_one};
use crate::hook::guard::shell::{Shape, Word};
use crate::hook::guard::{Denial, Rule};

fn deny(action: &str, label: &str) -> Denial {
    Denial::new(
        Rule::ConfigWrite,
        format!(
            "{action}: {label} is lint or hook config, which a Bash command may not change; \
             change it in an edit a human reviews"
        ),
    )
}

/// A redirection writing `target`, from `cwd`.
pub(super) fn redirect(target: &Word, cwd: &Cwd) -> Option<Denial> {
    let label = protected(&target.text, target.shape, cwd)?;
    Some(deny(
        &format!("a redirection writes {}", target.text),
        label,
    ))
}

/// Checks `<program> <args>`, run from `cwd`, for writes to protected config.
pub(super) fn check(program: &str, args: &[Word], cwd: &Cwd) -> Option<Denial> {
    match program {
        "tee" | "truncate" => args.iter().find_map(|arg| {
            let label = protected(&arg.text, arg.shape, cwd)?;
            Some(deny(&format!("{program} writes {}", arg.text), label))
        }),
        "rm" | "unlink" | "rmdir" | "shred" | "chmod" | "chown" | "chgrp" => {
            args.iter().find_map(|arg| {
                let label = contains_protected(&arg.text, arg.shape, cwd)?;
                Some(deny(&format!("{program} changes {}", arg.text), label))
            })
        }
        "sed" => sed(args, cwd),
        "cp" | "install" | "ln" => copy(program, args, false, cwd),
        "mv" => copy(program, args, true, cwd),
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
            } else if let Some(long) = Long::parse(text) {
                // `--in-place[=SUFFIX]`, `--expression SCRIPT`, `--file FILE`, abbreviated or not.
                sed.in_place |= long.is("--in-place");
                if long.is("--expression") || long.is("--file") {
                    sed.script_given = true;
                    if long.value.is_none() {
                        rest = skip_one(rest);
                    }
                }
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

fn sed(args: &[Word], cwd: &Cwd) -> Option<Denial> {
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
        let label = protected(&file.text, file.shape, cwd)?;
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
            } else if let Some(long) = Long::parse(text) {
                // `--target-directory DIR` and `--suffix SUFFIX`, abbreviated or not.
                if long.is("--target-directory") {
                    copy.target_dir = match long.value {
                        Some(dir) => Some((dir, first.shape)),
                        None => tail.first().map(|dir| (dir.text.as_str(), dir.shape)),
                    };
                }
                let valued = long.is("--target-directory") || long.is("--suffix");
                if valued && long.value.is_none() {
                    rest = skip_one(rest);
                }
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

fn copy(program: &str, args: &[Word], moves: bool, cwd: &Cwd) -> Option<Denial> {
    let copy = CopyArgs::parse(args);
    // A source lands at `<dest>/<its name>` when the destination is a directory: always for
    // `-t`, and, since the guard cannot look, when the destination ends in `/` or names a
    // directory that holds config.
    let ((dest, dest_shape), sources, into_dir) = if let Some(dir) = copy.target_dir {
        (dir, &copy.operands[..], true)
    } else {
        let (last, sources) = copy.operands.split_last()?;
        let named_dir = last.text.ends_with(['/', '\\'])
            || last_component(&last.text).is_some_and(|name| {
                CONTAINERS
                    .iter()
                    .any(|(dir, _)| dir.eq_ignore_ascii_case(name))
            });
        ((last.text.as_str(), last.shape), sources, named_dir)
    };
    if sources.is_empty() {
        return None;
    }
    if let Some(label) = protected(dest, dest_shape, cwd) {
        return Some(deny(&format!("{program} writes {dest}"), label));
    }
    sources.iter().find_map(|source| {
        if moves && let Some(label) = contains_protected(&source.text, source.shape, cwd) {
            return Some(deny(&format!("{program} moves {}", source.text), label));
        }
        let joined = format!("{dest}/{}", basename(&source.text));
        let label = protected(&joined, dest_shape.max(source.shape), cwd).filter(|_| into_dir)?;
        Some(deny(&format!("{program} writes {joined}"), label))
    })
}
