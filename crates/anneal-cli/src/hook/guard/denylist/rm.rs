//! `rm` with both recursive and force flags, outside the temporary and scratch directories.

use std::path::Path;

use super::wrappers::is_option;
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
    for word in args {
        let text = word.text.as_str();
        if !(options && is_option(word)) {
            targets.push(word);
            continue;
        }
        match text {
            "--" => options = false,
            "--recursive" => recursive = true,
            "--force" => force = true,
            long if long.starts_with("--") => {}
            short => {
                recursive |= short.contains(['r', 'R']);
                force |= short.contains('f');
            }
        }
    }
    if !(recursive && force) {
        return None;
    }
    let deny = |reason: String| Some(Denial::new(Rule::RecursiveForceDelete, reason));
    if appended {
        return deny(format!(
            "rm -rf with operands from xargs or find -exec cannot be checked; {ALLOWED}"
        ));
    }
    targets.into_iter().find_map(|target| {
        let why = unsafe_target(target, context)?;
        deny(format!("rm -rf {}: {why}; {ALLOWED}", target.text))
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
