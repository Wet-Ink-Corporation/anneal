//! A spec's `Tree:` line (#77): the design tree the spec was written from, named in the
//! header (the lines before the first `##` section) as a plan names its spec, and read
//! relative to the spec's directory. [`link`] reads the line; [`check_file`] is the edge that
//! reads the tree it names. `plan approve` reads the link through [`tree_link`].

use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use super::markdown::{Document, Line};
use super::{Rule, SpecFinding, header};
use crate::error_chain::reason;
use crate::grill::{self, tree::DesignTree};
use crate::lint::plan::parse::{key_value, spec_target};
use crate::lint::plan::spec_path;

/// The key of the line, matched in any letter case.
const KEY: &str = "Tree";

/// What a spec's header says about its design tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeLink {
    /// The line names a path, as written: relative to the spec's directory, or absolute.
    Named {
        /// The path as written.
        target: String,
        /// The `Tree:` line.
        line: usize,
    },
    /// The line names no file: its value is empty, or still the template's `<placeholder>`.
    Unnamed {
        /// The `Tree:` line.
        line: usize,
    },
}

impl TreeLink {
    /// The `Tree:` line's number.
    #[must_use]
    pub const fn line(&self) -> usize {
        match self {
            Self::Named { line, .. } | Self::Unnamed { line } => *line,
        }
    }
}

/// The first `Tree:` line of `header`, and a finding for a second one and for one with an
/// empty value. A value that is wholly a `<placeholder>`, as the template ships it, is
/// [`TreeLink::Unnamed`] with no finding here, as `lint spec` reports no other placeholder;
/// `plan approve` refuses it. No `Tree:` line is `None` and no finding: specs written before
/// the line existed still lint clean.
pub(super) fn link<'d>(
    file: &str,
    header: impl Iterator<Item = &'d Line<'d>>,
) -> (Option<TreeLink>, Vec<SpecFinding>) {
    let mut found = None;
    let mut findings = Vec::new();
    for line in header {
        let Some((_, value)) =
            key_value(&line.text).filter(|(key, _)| key.eq_ignore_ascii_case(KEY))
        else {
            continue;
        };
        if found.is_some() {
            findings.push(Rule::Tree.at(file, line.number).finding(
                "a second `Tree:` line; a spec is written from one design tree",
                "keep one `Tree:` line.",
            ));
            continue;
        }
        found = Some(match spec_target(value) {
            Some(target) if !is_placeholder(target) => TreeLink::Named {
                target: target.to_owned(),
                line: line.number,
            },
            Some(_) => TreeLink::Unnamed { line: line.number },
            None => {
                findings.push(Rule::Tree.at(file, line.number).finding(
                    "the `Tree:` line names no file",
                    "name the design tree the spec was written from, relative to this spec: \
                     `Tree: <path>`.",
                ));
                TreeLink::Unnamed { line: line.number }
            }
        });
    }
    (found, findings)
}

/// The design tree the header of the spec `text` links.
///
/// Its first `Tree:` line, read as `lint spec` reads it; `None` when the spec has no `Tree:`
/// line. `plan approve` needs the linked tree to be confirmed.
#[must_use]
pub fn tree_link(text: &str) -> Option<TreeLink> {
    let doc = Document::parse(text);
    let sections = doc.sections();
    link("", header(&doc, &sections)).0
}

/// The path of the design tree a spec at `spec` links as `target`.
///
/// An absolute `target` as written, otherwise `target` joined to the spec's directory with
/// `/`, as a plan's `Spec:` line is resolved against the plan.
#[must_use]
pub fn tree_path(spec: &str, target: &str) -> PathBuf {
    PathBuf::from(spec_path(spec, target, MAIN_SEPARATOR))
}

/// Whether `target` is a template placeholder: wholly inside `<` and `>`.
fn is_placeholder(target: &str) -> bool {
    target.starts_with('<') && target.ends_with('>')
}

/// The finding for the design tree at `path`, named on line `line` of the spec `file`, when it
/// cannot be read or is not a valid design tree (read as `anneal grill` reads it, relative to
/// the working directory); `None` when it is one.
pub(super) fn check_file(file: &str, line: usize, path: &Path) -> Option<SpecFinding> {
    let at = Rule::Tree.at(file, line);
    let shown = path.display();
    let bytes = match grill::read_tree(Path::new(""), path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Some(at.finding(
                format!("the design tree cannot be read: {}", reason(&error)),
                "name the design tree the spec was written from, relative to this spec.",
            ));
        }
    };
    let problems = DesignTree::parse(&bytes).err()?;
    let first = problems
        .first()
        .map_or_else(String::new, |problem| format!(": {problem}"));
    let more = problems.len().saturating_sub(1);
    let more = if more > 0 {
        format!(" (and {more} more)")
    } else {
        String::new()
    };
    Some(at.finding(
        format!("{shown} is not a valid design tree{first}{more}"),
        format!("run `anneal grill frontier --tree {shown}` and fix what it reports."),
    ))
}

#[cfg(test)]
mod tests {
    use super::{TreeLink, tree_link, tree_path};
    use crate::lint::spec::tests::{CLEAN, CONSTITUTION, found};
    use crate::lint::spec::{Rule, Source, check};

    /// [`CLEAN`] with `lines` under its title.
    fn linked(lines: &str) -> String {
        CLEAN.replacen("\n\n## Problem", &format!("\n\n{lines}\n\n## Problem"), 1)
    }

    fn checked(text: &str) -> crate::lint::spec::Report {
        let constitution = Source {
            file: "constitution.md",
            text: CONSTITUTION,
        };
        check(
            Source {
                file: "spec.md",
                text,
            },
            None,
            Some(constitution),
        )
    }

    #[test]
    fn a_tree_line_is_read_in_every_form_a_spec_line_is() {
        for line in [
            "Tree: trees/x.json",
            "- Tree: `trees/x.json`",
            "**Tree:** [the tree](trees/x.json)",
            "tree: trees/x.json",
        ] {
            assert_eq!(
                tree_link(&linked(line)),
                Some(TreeLink::Named {
                    target: "trees/x.json".to_owned(),
                    line: 3
                }),
                "{line}"
            );
        }
    }

    #[test]
    fn the_shipped_template_has_a_tree_line_still_to_be_filled_in() {
        let template = include_str!("../../../../../plugin/playbooks/spec.template.md");

        assert!(
            matches!(tree_link(template), Some(TreeLink::Unnamed { .. })),
            "the template's `Tree:` line is a placeholder"
        );
    }

    #[test]
    fn a_spec_without_a_tree_line_links_nothing_and_has_no_finding() {
        let report = checked(CLEAN);

        assert_eq!(
            (&report.tree, found(&report)),
            (&None, vec![]),
            "a spec from before #77"
        );
    }

    #[test]
    fn a_tree_line_after_the_header_is_not_read() {
        let text = CLEAN.replacen("Analysts.", "Analysts.\n\nTree: trees/x.json", 1);

        assert_eq!(tree_link(&text), None, "only the header is read");
    }

    #[test]
    fn the_template_placeholder_links_no_tree_and_is_no_finding() {
        let report = checked(&linked("Tree: <the design tree, relative to this spec>"));

        assert_eq!(
            (&report.tree, found(&report)),
            (&Some(TreeLink::Unnamed { line: 3 }), vec![]),
            "unnamed, as other placeholders are not findings"
        );
    }

    #[test]
    fn an_empty_tree_line_is_a_finding() {
        let report = checked(&linked("Tree:"));

        assert_eq!(
            (&report.tree, found(&report)),
            (
                &Some(TreeLink::Unnamed { line: 3 }),
                vec![(Rule::Tree, 3, None)]
            ),
            "names no file"
        );
    }

    #[test]
    fn a_second_tree_line_is_a_finding_and_the_first_is_kept() {
        let report = checked(&linked("Tree: a.json\nTree: b.json"));

        assert_eq!(
            (&report.tree, found(&report)),
            (
                &Some(TreeLink::Named {
                    target: "a.json".to_owned(),
                    line: 3
                }),
                vec![(Rule::Tree, 4, None)]
            ),
            "one tree per spec"
        );
    }

    #[test]
    fn a_tree_path_is_read_relative_to_the_spec_s_directory() {
        assert_eq!(
            (
                tree_path("docs/specs/x.md", "../trees/x.json"),
                tree_path("x.md", "x.json"),
                tree_path("docs/x.md", "/srv/x.json"),
            ),
            (
                "docs/specs/../trees/x.json".into(),
                "x.json".into(),
                "/srv/x.json".into()
            ),
            "joined to the directory, an absolute path as written"
        );
    }
}
