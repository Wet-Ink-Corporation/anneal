//! The text of a brief and of its review package. Pure: `brief build` resolves the range and
//! reads the diff, then hands everything here.
//!
//! Both put what stays the same first (the title and the nine fields, in table order) and what
//! changes on every build last (commits, `cacheTtl`, the time), so a prompt that starts with
//! the stable part keeps its cache.

use crate::brief::fields::{FIELDS, Field, Fill};
use crate::brief::ticket::Ticket;
use crate::brief::{brief_file, package_file};

/// A field value given to `brief build --field key=text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Given<'a> {
    /// The field's key.
    pub(crate) key: &'a str,
    /// Its text.
    pub(crate) text: &'a str,
}

/// The resolved range the brief is built from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Range<'a> {
    /// The base as given (`origin/main`).
    pub(crate) base: &'a str,
    /// The base's full commit id.
    pub(crate) base_commit: &'a str,
    /// The head's full commit id.
    pub(crate) head: &'a str,
}

/// Everything a brief and its review package are written from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Brief<'a> {
    pub(crate) ticket: &'a Ticket,
    /// The fields given, each key at most once.
    pub(crate) given: &'a [Given<'a>],
    /// The text of a field whose default is [`Fill::Repository`].
    pub(crate) repository: &'a str,
    pub(crate) range: Range<'a>,
    /// The subagent waits on CI, so its prompt cache should outlive the wait.
    pub(crate) waits_on_ci: bool,
    /// When the brief was built, RFC 3339.
    pub(crate) built: &'a str,
}

impl Brief<'_> {
    /// The text of `field`: the given text with its appendix, else its default (an empty
    /// field is a comment naming its purpose, which `lint brief` does not count as content).
    fn text_of(&self, field: &Field) -> String {
        let given = self.given.iter().find(|given| given.key == field.key);
        match (given, field.default) {
            (Some(given), _) => match field.appendix {
                Some(appendix) => format!("{}\n\n{appendix}", given.text.trim()),
                None => given.text.trim().to_owned(),
            },
            (None, Fill::Text(text)) => text.to_owned(),
            (None, Fill::Repository) => self.repository.to_owned(),
            (None, Fill::Empty) => format!(
                "<!-- {} Fill in, or rebuild with --field {}=<text>. -->",
                field.purpose, field.key
            ),
        }
    }

    /// The nine fields as sections whose headings start with `marker`.
    fn fields(&self, marker: &str) -> String {
        FIELDS
            .iter()
            .map(|field| {
                [
                    marker,
                    " ",
                    field.heading,
                    "\n\n",
                    &self.text_of(field),
                    "\n\n",
                ]
                .concat()
            })
            .collect()
    }

    /// The section that changes on every build: the commits, `cacheTtl` when `cache_ttl`,
    /// and the build time.
    fn volatile(&self, cache_ttl: bool) -> String {
        let Range {
            base,
            base_commit,
            head,
        } = self.range;
        let ttl = if cache_ttl { "- cacheTtl: 1h\n" } else { "" };
        format!(
            "## Volatile\n\nChanges on every build; everything above is stable.\n\n\
             - Base: `{base}` at `{base_commit}`\n- Head: `{head}`\n{ttl}- Built: {}\n",
            self.built
        )
    }
}

/// The brief: title, review package path, the nine fields, then the volatile section.
pub(crate) fn brief(inputs: &Brief<'_>) -> String {
    let ticket = inputs.ticket;
    [
        format!(
            "# Brief: {ticket}\n\nReview package: `{}`\n\n",
            package_file(ticket)
        ),
        inputs.fields("##"),
        inputs.volatile(inputs.waits_on_ci),
    ]
    .concat()
}

/// The review package: the contract (the nine fields) and the diff, then the volatile
/// section. `diff` is `None` when it was over the cap; the package then names the command
/// that shows it instead of a partial diff.
pub(crate) fn review_package(inputs: &Brief<'_>, diff: Option<&str>) -> String {
    let ticket = inputs.ticket;
    let Range {
        base,
        base_commit,
        head,
    } = inputs.range;
    let body = match diff {
        Some("") => "No change yet: the head is the base.\n\n".to_owned(),
        Some(diff) => {
            let fence = "`".repeat(longest_backtick_run(diff).saturating_add(1).max(3));
            let newline = if diff.ends_with('\n') { "" } else { "\n" };
            format!("{fence}diff\n{diff}{newline}{fence}\n\n")
        }
        None => format!(
            "The diff is over the size cap and is not included: run \
             `git diff {base_commit}...{head}`.\n\n"
        ),
    };
    [
        format!(
            "# Review package: {ticket}\n\nBrief: `{}`\n\n## Contract\n\n",
            brief_file(ticket)
        ),
        inputs.fields("###"),
        format!("## Diff\n\nThe change on top of `{base}`: `git diff {base}...HEAD`.\n\n"),
        body,
        inputs.volatile(false),
    ]
    .concat()
}

/// The longest run of backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Brief, Given, Range, brief, review_package};
    use crate::brief::fields::FIELDS;
    use crate::brief::ticket::Ticket;
    use crate::lint::brief::{Rule, check};

    const BASE: &str = "1111111111111111111111111111111111111111";
    const HEAD: &str = "2222222222222222222222222222222222222222";

    fn every_field() -> Vec<Given<'static>> {
        FIELDS
            .iter()
            .map(|field| Given {
                key: field.key,
                text: "filled",
            })
            .collect()
    }

    fn inputs<'a>(ticket: &'a Ticket, given: &'a [Given<'a>], waits_on_ci: bool) -> Brief<'a> {
        Brief {
            ticket,
            given,
            repository: "/work/anneal, from origin/main",
            range: Range {
                base: "origin/main",
                base_commit: BASE,
                head: HEAD,
            },
            waits_on_ci,
            built: "2026-10-12T09:00:00Z",
        }
    }

    #[test]
    fn the_fields_come_first_in_table_order_and_the_commits_and_time_last() {
        let ticket: Ticket = "36".parse().expect("a ticket");
        let given = every_field();

        let text = brief(&inputs(&ticket, &given, false));

        let headings: Vec<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix("## "))
            .collect();
        let mut expected: Vec<&str> = FIELDS.iter().map(|field| field.heading).collect();
        expected.push("Volatile");
        assert_eq!(headings, expected, "nine fields, then the volatile section");
        let volatile = text.find("## Volatile").expect("a volatile section");
        for late in [BASE, HEAD, "2026-10-12T09:00:00Z"] {
            let at = text.find(late).expect("written");
            assert!(at > volatile, "{late} only after the stable part");
        }
        assert_eq!(
            check(&text),
            Vec::new(),
            "every field given: it lints clean"
        );
    }

    #[test]
    fn cache_ttl_is_written_only_for_a_subagent_that_waits_on_ci() {
        let ticket: Ticket = "36".parse().expect("a ticket");
        let given = every_field();

        let waiting = brief(&inputs(&ticket, &given, true));
        let not_waiting = brief(&inputs(&ticket, &given, false));

        assert!(
            waiting.contains("\n- cacheTtl: 1h\n"),
            "a fix loop waiting on CI: {waiting}"
        );
        assert!(!not_waiting.contains("cacheTtl"), "{not_waiting}");
    }

    #[test]
    fn a_field_not_given_is_a_placeholder_or_its_default_and_edits_carry_the_no_revert_rule() {
        let ticket: Ticket = "T1".parse().expect("a ticket");
        let given = [
            Given {
                key: "goal",
                text: "Ship it.",
            },
            Given {
                key: "edits-allowed",
                text: "yes",
            },
        ];

        let text = brief(&inputs(&ticket, &given, false));

        let empty: Vec<&str> = check(&text)
            .into_iter()
            .filter(|finding| finding.rule == Rule::Empty)
            .map(|finding| finding.field)
            .collect();
        assert_eq!(
            empty,
            [
                "Owned scope",
                "Do not touch",
                "Invariants",
                "Success criteria",
                "Verification"
            ],
            "Repository and Return contract have defaults; the rest wait for their author"
        );
        assert!(
            text.contains("## Edits allowed\n\nyes\n\nNever revert"),
            "the no-revert rule follows the answer: {text}"
        );
        assert!(
            text.contains("## Repository\n\n/work/anneal, from origin/main\n"),
            "{text}"
        );
    }

    #[test]
    fn the_review_package_holds_the_contract_and_the_diff_in_a_fence_longer_than_its_backticks() {
        let ticket: Ticket = "36".parse().expect("a ticket");
        let given = every_field();
        let diff = "diff --git a/x.md b/x.md\n+````\n";

        let text = review_package(&inputs(&ticket, &given, false), Some(diff));

        assert!(text.contains("## Contract\n"), "{text}");
        assert!(text.contains("### Success criteria\n\nfilled\n"), "{text}");
        assert!(
            text.contains(&format!("`````diff\n{diff}`````\n")),
            "a five-backtick fence around a diff with four: {text}"
        );
        assert!(
            text.find("## Volatile").expect("volatile") > text.find("## Diff").expect("diff"),
            "commits and time last"
        );
    }

    #[test]
    fn a_diff_over_the_cap_is_named_with_the_command_that_shows_it() {
        let ticket: Ticket = "36".parse().expect("a ticket");
        let given = every_field();

        let text = review_package(&inputs(&ticket, &given, false), None);

        assert!(
            text.contains(&format!("git diff {BASE}...{HEAD}")),
            "the reader can run it: {text}"
        );
        assert!(!text.contains("```diff"), "no partial diff");
    }
}
