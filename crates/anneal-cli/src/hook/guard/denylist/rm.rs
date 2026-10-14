//! `rm` with both recursive and force flags, outside the temporary and scratch directories.
//!
//! An argument whose text outside its variables and substitutions is nothing (`$F`,
//! `"$FLAGS"`) or an option cluster (`"$b"-rf`, `-$x`, or what `"$b"{-rf,/}` expands to) may
//! hold the flags, so when another operand follows it, it is taken as `-rf`. An operand after
//! `--` is never an option, so it never holds them.

use std::path::Path;

use super::wrappers::{is_long, may_be_option};
use super::{Context, normalize};
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
            .position(|target| may_hide_flags(target))
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

/// Whether `word`'s value may be an option cluster holding the flags: its text outside its
/// expansions is nothing (`$F`, `"$a$b"`) or an option cluster (`"$b"-rf`, `-$x`), which the
/// expansions may leave as it is or fill in.
fn may_hide_flags(word: &Word) -> bool {
    if word.expansions.is_empty() {
        return false;
    }
    // A range outside the text (which the lexer never makes) is taken as possibly the flags,
    // which can only add denials.
    let Some(outside) = word.outside_expansions() else {
        return true;
    };
    let mut chars = outside.into_iter().flat_map(str::chars);
    match chars.next() {
        None => true,
        Some('-') => chars.all(|c| c.is_ascii_alphanumeric() || c == '-'),
        Some(_) => false,
    }
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
