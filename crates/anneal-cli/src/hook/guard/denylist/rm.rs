//! `rm` with both recursive and force flags, outside the temporary and scratch directories.
//!
//! An argument one of whose values may be an option cluster may hold the flags
//! ([`readings::may_start_option_cluster`]): a variable or substitution first (`$F`, `"$a$b"`,
//! `"$b"rf` with `b=-`), a glob first that may match a file named `-rf` (`?rf`, `[-]rf`), or a
//! written `-` (`-$x`), followed only by letters, digits and `-`. When another operand follows
//! it, it is taken as `-rf`. An operand after `--` is never an option, so it never holds them.

use std::path::Path;

use super::wrappers::{is_long, may_be_option};
use super::{Context, normalize, readings};
use crate::hook::guard::shell::{Shape, Word};
use crate::hook::guard::{Denial, Rule};

/// What a denial tells the agent may be force-deleted instead.
const ALLOWED: &str = "only literal absolute paths under $TMPDIR, /tmp, /var/folders or \
                       $CLAUDE_PROJECT_DIR/.scratch may be force-deleted recursively";

/// Checks `rm <args>`; `appended` when `xargs` or `find -exec` adds operands.
pub(super) fn check(args: &[Word], context: &Context, appended: bool) -> Option<Denial> {
    let (mut recursive, mut force) = (false, false);
    let mut targets = Vec::new();
    let mut options = true;
    // How many operands came before `--`: after it none is an option, so none hides the flags.
    let mut before_dashes = None;
    for word in args {
        let text = word.text.as_str();
        if !(options && may_be_option(word)) {
            targets.push(word);
            continue;
        }
        match text {
            "--" => {
                options = false;
                before_dashes = Some(targets.len());
            }
            long if long.starts_with("--") => {
                recursive |= is_long(long, "--recursive");
                force |= is_long(long, "--force");
            }
            short => {
                recursive |= short.contains(['r', 'R']);
                force |= short.contains('f');
            }
        }
    }
    let mut flags = None;
    if !(recursive && force) {
        let hidden = targets
            .iter()
            .take(before_dashes.unwrap_or(targets.len()))
            .position(|target| readings::may_start_option_cluster(target))
            .filter(|_| targets.len() >= 2)?;
        flags = Some(targets.remove(hidden));
    }
    let deny = |reason: String| Some(Denial::new(Rule::RecursiveForceDelete, reason));
    if appended {
        return deny(format!(
            "rm -rf with operands from xargs or find -exec cannot be checked; {ALLOWED}"
        ));
    }
    targets.into_iter().find_map(|target| {
        let why = unsafe_target(target, context)?;
        match flags {
            Some(flags) => deny(format!(
                "rm {} {}: {} may be -rf, and {why}; {ALLOWED}",
                flags.text, target.text, flags.text
            )),
            None => deny(format!("rm -rf {}: {why}; {ALLOWED}", target.text)),
        }
    })
}

/// Why `target` may not be force-deleted, or `None` when it may.
fn unsafe_target(target: &Word, context: &Context) -> Option<&'static str> {
    match target.shape {
        Shape::Dynamic => {
            return Some("it holds a variable or substitution known only at run time");
        }
        Shape::Pattern => return Some("it holds a glob, a brace or ~"),
        Shape::Literal => {}
    }
    let path = Path::new(&target.text);
    if !path.has_root() {
        return Some("it is not an absolute path");
    }
    if context.is_disposable(&normalize(path)) {
        None
    } else {
        Some("it is outside the temporary and scratch directories")
    }
}
