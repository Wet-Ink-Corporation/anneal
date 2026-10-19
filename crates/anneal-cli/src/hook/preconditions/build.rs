//! Build mode's preconditions, checked in the order the story lists them: the spec, the grill
//! confirmation, the clean tree, then the plan's approval. The first that fails is the denial.

use std::path::{Path, PathBuf};

use anneal_ledger::Store;

use super::{Check, Denial, unjudged};
use crate::content_hash::sha256;
use crate::grill;
use crate::hook::reason;
use crate::lint::plan as plan_lint;
use crate::lint::read_whole_bytes;
use crate::lint::spec::{self as spec_lint, DEFAULT_CONSTITUTION, DEFAULT_GLOSSARY, SpecArgs};
use crate::mode::BuildMode;
use crate::mode::scope::{self, PathKind};
use crate::plan::{self, Status};

/// Whether an edit of `relative` (relative to the worktree root) in build mode entered with
/// `build` is gated: it is source, not a test, the plan or tree, scratch or a document.
pub(super) fn is_gated(build: &BuildMode, relative: &str) -> bool {
    let artifacts = [build.plan.as_str(), build.tree.as_str()];
    scope::classify(relative, &artifacts) == PathKind::Source
}

/// The first build-mode precondition that does not hold in the worktree `root`, entered with
/// `build`; `None` when every one holds. The same for every gated file of the worktree.
pub(super) fn check(store: &Store, root: &Path, build: &BuildMode) -> Option<Denial> {
    let plan_path = root.join(&build.plan);
    let bytes = match read_whole_bytes(&plan_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Some(Denial::new(
                Check::Plan,
                format!(
                    "build mode: the plan {} cannot be read ({}); restore it, or run `anneal \
                     mode build --plan <plan> --tree {}`",
                    build.plan,
                    reason(&error),
                    build.tree
                ),
            ));
        }
    };
    // The spec is not exempt by name: the plan's `Spec:` line is editable, so naming a source
    // file there would unlock it. A spec is editable as the prose document it is (`*.md`,
    // `*.txt`, `*.rst`, allowed above), and any other file it names is source like the rest.
    let spec = plan_lint::examine(&plan_path, &bytes).spec;
    let spec_relative = spec
        .as_deref()
        .and_then(|spec| lexically_relative(root, spec));
    let spec = spec.map(|path| SpecFile {
        shown: spec_relative.unwrap_or_else(|| path.display().to_string()),
        path,
    });
    spec_check(root, &build.plan, spec)
        .or_else(|| grill_check(store, root, &build.tree))
        .or_else(|| clean_check(build))
        .or_else(|| plan_check(store, &build.plan, &bytes))
}

/// The spec a plan links: its path, and how a denial names it (relative to the worktree root
/// when it is inside it).
struct SpecFile {
    path: PathBuf,
    shown: String,
}

/// `path` relative to `root`, comparing the text as `scope` does; both name the worktree as git
/// printed it, so no symlink needs resolving.
fn lexically_relative(root: &Path, path: &Path) -> Option<String> {
    let root = scope::normalise(&root.to_string_lossy());
    let path = scope::normalise(&path.to_string_lossy());
    scope::relative(&root, &path).map(str::to_owned)
}

/// The spec precondition: the plan `plan` links `spec`, which passes `lint spec` with the
/// glossary and constitution at the worktree root, when they exist.
fn spec_check(root: &Path, plan: &str, spec: Option<SpecFile>) -> Option<Denial> {
    let Some(SpecFile { path, shown }) = spec else {
        return Some(Denial::new(
            Check::Spec,
            format!(
                "build mode: the plan {plan} links no spec; add its `Spec:` line, then run \
                 `anneal lint plan {plan}`"
            ),
        ));
    };
    let denial = |problem: String| {
        Some(Denial::new(
            Check::Spec,
            format!("build mode: the spec {shown} {problem}, then run `anneal lint spec {shown}`"),
        ))
    };
    if !path.is_file() {
        return denial("is not a file; write it".to_owned());
    }
    let args = SpecArgs {
        file: path,
        glossary: present(root, DEFAULT_GLOSSARY),
        constitution: present(root, DEFAULT_CONSTITUTION),
    };
    match spec_lint::lint(&args) {
        Ok(outcome) if outcome.report.findings.is_empty() => None,
        Ok(outcome) => denial(format!(
            "fails `anneal lint spec` with {} finding(s); fix them",
            outcome.report.findings.len()
        )),
        Err(error) => denial(format!("cannot be read ({}); fix it", reason(&error))),
    }
}

/// `root/name` when it is a file: the default glossary or constitution, read from the worktree
/// root as `lint spec` reads it from where it runs.
fn present(root: &Path, name: &str) -> Option<PathBuf> {
    Some(root.join(name)).filter(|path| path.is_file())
}

/// The grill precondition: one of the latest confirmations has the design tree's current hash.
fn grill_check(store: &Store, root: &Path, tree: &str) -> Option<Denial> {
    let command = format!("`anneal grill confirm --tree {tree} --quote \"<the human's words>\"`");
    let bytes = match grill::read_tree(root, Path::new(tree)) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Some(Denial::new(
                Check::Grill,
                format!(
                    "build mode: the design tree {tree} cannot be read ({}); restore it, then \
                     run {command}",
                    reason(&error)
                ),
            ));
        }
    };
    match grill::is_confirmed(store, &sha256(&bytes)) {
        Ok(true) => None,
        Ok(false) => Some(Denial::new(
            Check::Grill,
            format!(
                "build mode: the design tree {tree} is not confirmed at its current content; \
                 have the human confirm it, then run {command}"
            ),
        )),
        Err(error) => Some(unjudged("the ledger cannot be read", &error)),
    }
}

/// The clean precondition: the working tree was clean when build mode was entered.
fn clean_check(build: &BuildMode) -> Option<Denial> {
    (!build.clean).then(|| {
        Denial::new(
            Check::Clean,
            format!(
                "build mode: the working tree was not clean when build mode started; commit or \
                 stash the changes, then run `anneal mode build --plan {} --tree {}`",
                build.plan, build.tree
            ),
        )
    })
}

/// The plan precondition: the plan recorded as `plan` is approved at `bytes`, as `plan check`
/// reports.
fn plan_check(store: &Store, plan: &str, bytes: &[u8]) -> Option<Denial> {
    let problem = match plan::approval_of(store, plan, bytes) {
        Ok(Status::Current) => return None,
        Ok(Status::NotApproved) => "is not approved; have the human review it",
        Ok(Status::Stale) => "changed since it was approved; have the human review the change",
        Err(error) => return Some(unjudged("the plan's approval cannot be read", &error)),
    };
    Some(Denial::new(
        Check::Plan,
        format!("build mode: the plan {plan} {problem}, then run `anneal plan approve {plan}`"),
    ))
}
