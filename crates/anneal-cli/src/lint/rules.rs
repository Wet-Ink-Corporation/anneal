//! `anneal lint rules`: every rule in a rule document names an enforcer that exists (design
//! rule 10, "Name the enforcer", applied to rules a retro writes; `docs/skill-rules.md`).
//!
//! A rule document is markdown with a `## Rules` section. Each list item in it is one rule, and
//! names its enforcer after `Enforcer:` in backticks: `anneal <command> ...`, `hook <id>` or
//! `test <file>::<fn>` (see the private `enforcer` module). A rule with none, or whose enforcer cannot be found,
//! is an `enforcer` error. Findings use the lint finding model, so the envelope and exit codes
//! are `lint skills`' own.

pub(crate) mod enforcer;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::{Args, CommandFactory};

use crate::cli::Cli;
use crate::lint::finding::{LintFinding, Location};
use crate::lint::read_whole;
use crate::lint::skills::checker::Checker;
use crate::lint::skills::{LintError, read_hook_ids};
use crate::lint::spec::markdown::{Document, Item, items};
use enforcer::{Repository, Root, Unresolved};

/// The section that holds the rules (level 2, any letter case).
const RULES_SECTION: &str = "Rules";

/// The label an enforcer follows, in backticks.
const LABEL: &str = "Enforcer:";

/// The hooks metadata under the root, used when `--hooks` is not given and it exists.
const DEFAULT_HOOKS: [&str; 3] = ["plugin", "hooks", "hooks.metadata.json"];

/// What a rule with no enforcer, or an unknown one, should do.
const FIX: &str = "after `Enforcer:`, name in backticks the hook (`hook <id>`), the lint or \
                   command (`anneal <command> ...`) or the test (`test <file>::<fn>`) that \
                   enforces it; a rule nothing enforces is a wish.";

/// Arguments of `anneal lint rules`.
#[derive(Debug, Clone, Args)]
pub struct RulesArgs {
    /// Rule documents: markdown files whose `## Rules` section lists one rule per item, each
    /// naming its enforcer after `Enforcer:`
    #[arg(value_name = "PATH", required = true)]
    pub paths: Vec<PathBuf>,
    /// The repository root that `test <file>::<fn>` enforcers are relative to
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub root: PathBuf,
    /// hooks.metadata.json whose ids `hook <id>` enforcers name; defaults to
    /// plugin/hooks/hooks.metadata.json under --root, when that file exists
    #[arg(long, value_name = "FILE")]
    pub hooks: Option<PathBuf>,
}

/// What a run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Every finding, sorted by file, line and subject.
    pub findings: Vec<LintFinding>,
    /// How many rule documents were read.
    pub documents: usize,
}

/// Lints the rule documents of `args.paths`.
///
/// # Errors
/// [`LintError`] when a path or the root is missing or the wrong kind, a document is over the
/// read cap, or a file cannot be read. A rule without a findable enforcer is a finding, never
/// an error.
pub fn lint(args: &RulesArgs) -> Result<Outcome, LintError> {
    require_dir(&args.root)?;
    let root = Root::resolve(&args.root)?;
    let (hook_ids, mut findings) = hook_ids(args)?;
    let commands = Cli::command();
    let repository = Repository {
        root: &root,
        hook_ids: &hook_ids,
        commands: &commands,
    };
    for path in &args.paths {
        let text = read_whole(path)?;
        findings.extend(check_document(
            &path.display().to_string(),
            &text,
            &repository,
        )?);
    }
    findings.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    Ok(Outcome {
        findings,
        documents: args.paths.len(),
    })
}

/// The findings of one rule document, `file` being its name as given.
fn check_document(
    file: &str,
    text: &str,
    repository: &Repository<'_>,
) -> Result<Vec<LintFinding>, LintError> {
    let document = Document::parse(text);
    let sections: Vec<_> = document
        .sections()
        .into_iter()
        .filter(|section| section.title.eq_ignore_ascii_case(RULES_SECTION))
        .collect();
    let Some(first) = sections.first() else {
        return Ok(vec![LintFinding::error(
            Checker::Enforcer,
            Location::new(file, 0),
            "no `## Rules` section, so no rule is checked.",
            "list each rule as an item under `## Rules`, naming its enforcer.",
        )]);
    };
    let rules: Vec<Item> = sections
        .iter()
        .flat_map(|section| items(section.body))
        .collect();
    if rules.is_empty() {
        return Ok(vec![LintFinding::error(
            Checker::Enforcer,
            Location::new(file, first.line),
            "the Rules section lists no rule.",
            "list each rule as an item under `## Rules`, naming its enforcer.",
        )]);
    }
    let mut findings = Vec::new();
    for rule in &rules {
        findings.extend(check_rule(file, rule, repository)?);
    }
    Ok(findings)
}

/// The findings of one rule: none when it names at least one enforcer and each can be found.
fn check_rule(
    file: &str,
    rule: &Item,
    repository: &Repository<'_>,
) -> Result<Vec<LintFinding>, LintError> {
    let at = || Location::new(file, rule.line);
    let named: Vec<&str> = named_enforcers(&rule.text).collect();
    if named.is_empty() {
        return Ok(vec![LintFinding::error(
            Checker::Enforcer,
            at(),
            "this rule names no enforcer.",
            FIX,
        )]);
    }
    let mut findings = Vec::new();
    for span in named {
        let resolved = match enforcer::parse(span) {
            Ok(named) => enforcer::resolve(&named, repository)?,
            Err(why) => Err(why),
        };
        if let Err(why) = resolved {
            findings.push(unknown(at(), span, &why));
        }
    }
    Ok(findings)
}

fn unknown(at: Location, span: &str, why: &Unresolved) -> LintFinding {
    LintFinding::error(
        Checker::Enforcer,
        at,
        format!("enforcer `{span}` cannot be found: {why}."),
        FIX,
    )
    .with_subject(span)
}

/// The backticked span after each `Enforcer:` label of `text`.
fn named_enforcers(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices(LABEL).filter_map(|(at, _)| {
        let after = text.get(at..)?.strip_prefix(LABEL)?.trim_start();
        let (span, _) = after.strip_prefix('`')?.split_once('`')?;
        Some(span)
    })
}

/// The hook ids that `hook <id>` may name, plus a finding when the metadata does not parse.
fn hook_ids(args: &RulesArgs) -> Result<(BTreeSet<String>, Vec<LintFinding>), LintError> {
    if let Some(path) = &args.hooks {
        return read_hook_ids(path);
    }
    let default = args.root.join(DEFAULT_HOOKS.iter().collect::<PathBuf>());
    if default.is_file() {
        read_hook_ids(&default)
    } else {
        Ok((BTreeSet::new(), Vec::new()))
    }
}

/// `--root` must be an existing directory; either mistake is a usage error.
fn require_dir(path: &Path) -> Result<(), LintError> {
    if !path.exists() {
        return Err(LintError::MissingPath {
            path: path.to_path_buf(),
        });
    }
    if !path.is_dir() {
        return Err(LintError::WrongKind {
            path: path.to_path_buf(),
            expected: "a directory",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
