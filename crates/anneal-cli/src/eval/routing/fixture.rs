//! The routing fixture reader: a restricted YAML subset, read by hand (`evals/README.md`).
//!
//! A fixture has two top-level keys, each optional and each at most once:
//!
//! ```yaml
//! positives:
//!   - a prompt that must route to this skill
//! near_misses:
//!   other-skill:
//!     - a prompt that looks like this skill but belongs to other-skill
//! ```
//!
//! Items are scalars with the frontmatter parser's rules (plain, `"double"` or `'single'`
//! quoted; a plain item may not contain `: ` or start with a YAML indicator). Flow collections
//! (`[`, `{`), tabs, and anything else are a [`ParseError`] with the line it stopped at, so a
//! file this reader accepts reads the same under a real YAML parser (Tier 2, #40).

use std::collections::BTreeSet;

use crate::lint::skills::frontmatter::{ParseError, unquote};

/// One prompt and its 1-based line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prompt {
    /// The prompt text, unquoted.
    pub(crate) text: String,
    /// 1-based line of its `- ` item.
    pub(crate) line: usize,
}

/// A prompt that looks like the fixture's skill but belongs to `owner`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NearMiss {
    /// The skill the prompt belongs to.
    pub(crate) owner: String,
    /// The prompt.
    pub(crate) prompt: Prompt,
}

/// A parsed routing fixture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Fixture {
    /// Prompts that must rank the fixture's skill first.
    pub(crate) positives: Vec<Prompt>,
    /// Prompts whose owner must outrank the fixture's skill, in file order.
    pub(crate) near_misses: Vec<NearMiss>,
    /// Line of the `positives:` key, or 0 when absent.
    pub(crate) positives_line: usize,
    /// Line of the `near_misses:` key, or 0 when absent.
    pub(crate) near_misses_line: usize,
}

/// Parses a routing fixture.
///
/// # Errors
/// [`ParseError`] with the 1-based line where the text leaves the supported subset.
pub(crate) fn parse(text: &str) -> Result<Fixture, ParseError> {
    let mut reader = Reader::default();
    for (index, line) in text.lines().enumerate() {
        let line_no = index.saturating_add(1);
        let content = line.trim_start_matches(' ');
        if content.trim().is_empty() || content.starts_with('#') {
            continue;
        }
        if content.starts_with('\t') {
            return Err(error(line_no, "tabs cannot indent YAML; use spaces"));
        }
        let indent = line.len().saturating_sub(content.len());
        reader.line(indent, content.trim_end(), line_no)?;
    }
    Ok(reader.fixture)
}

/// The top-level key being read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Section {
    /// Before the first key.
    #[default]
    Start,
    /// Under `positives:`.
    Positives,
    /// Under `near_misses:`.
    NearMisses,
}

#[derive(Debug, Default)]
struct Reader {
    fixture: Fixture,
    section: Section,
    /// The indentation of the current list's items, once its first item is read.
    item_indent: Option<usize>,
    /// The indentation of `owner:` keys under `near_misses:`, once the first is read.
    owner_indent: Option<usize>,
    /// The current near-miss owner.
    owner: Option<String>,
    owners: BTreeSet<String>,
}

impl Reader {
    fn line(&mut self, indent: usize, content: &str, line: usize) -> Result<(), ParseError> {
        if indent == 0 {
            return self.section_key(content, line);
        }
        match self.section {
            Section::Start => Err(error(line, "unexpected indentation")),
            Section::Positives => {
                let text = self.item(indent, content, line)?;
                self.fixture.positives.push(Prompt { text, line });
                Ok(())
            }
            Section::NearMisses => self.near_miss_line(indent, content, line),
        }
    }

    fn section_key(&mut self, content: &str, line: usize) -> Result<(), ParseError> {
        let Some((key, rest)) = content.split_once(':') else {
            return Err(error(line, "expected `positives:` or `near_misses:`"));
        };
        let (section, key_line) = match key {
            "positives" => (Section::Positives, &mut self.fixture.positives_line),
            "near_misses" => (Section::NearMisses, &mut self.fixture.near_misses_line),
            _ => {
                return Err(error(
                    line,
                    "unknown key; expected `positives:` or `near_misses:`",
                ));
            }
        };
        if !rest.trim().is_empty() {
            return Err(error(
                line,
                "a section key takes an indented list, not a value",
            ));
        }
        if *key_line != 0 {
            return Err(error(line, "duplicate key"));
        }
        *key_line = line;
        self.section = section;
        self.item_indent = None;
        Ok(())
    }

    fn near_miss_line(
        &mut self,
        indent: usize,
        content: &str,
        line: usize,
    ) -> Result<(), ParseError> {
        let owner_indent = *self.owner_indent.get_or_insert(indent);
        if indent == owner_indent {
            return self.owner_key(content, line);
        }
        let Some(owner) = self.owner.as_ref().filter(|_| indent > owner_indent) else {
            return Err(error(line, "inconsistent indentation"));
        };
        // Each near miss owns its owner's name: genuine duplication, one per prompt (H-08).
        let owner = owner.clone();
        let text = self.item(indent, content, line)?;
        self.fixture.near_misses.push(NearMiss {
            owner,
            prompt: Prompt { text, line },
        });
        Ok(())
    }

    fn owner_key(&mut self, content: &str, line: usize) -> Result<(), ParseError> {
        let owner = content
            .strip_suffix(':')
            .filter(|name| is_skill_name(name))
            .ok_or_else(|| error(line, "expected `<owner-skill>:` followed by its prompts"))?;
        if !self.owners.insert(owner.to_owned()) {
            return Err(error(line, "duplicate owner"));
        }
        self.owner = Some(owner.to_owned());
        self.item_indent = None;
        Ok(())
    }

    /// One `- prompt` item at `indent`, which must match the list's other items.
    fn item(&mut self, indent: usize, content: &str, line: usize) -> Result<String, ParseError> {
        if *self.item_indent.get_or_insert(indent) != indent {
            return Err(error(line, "inconsistent indentation"));
        }
        let rest = content
            .strip_prefix('-')
            .ok_or_else(|| error(line, "expected a `- prompt` item"))?;
        if !(rest.is_empty() || rest.starts_with(' ')) {
            return Err(error(line, "expected a `- prompt` item"));
        }
        let scalar = rest.trim();
        if scalar.starts_with(['[', '{']) {
            return Err(error(line, "a flow collection is not a prompt; quote it"));
        }
        let text = unquote(scalar, line)?;
        if text.trim().is_empty() {
            return Err(error(line, "empty prompt"));
        }
        Ok(text)
    }
}

/// A skill name as `lint skills` requires it: lowercase letters, digits and hyphens.
fn is_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

const fn error(line: usize, reason: &'static str) -> ParseError {
    ParseError { line, reason }
}

#[cfg(test)]
mod tests {
    use super::{Fixture, NearMiss, Prompt, parse};
    use crate::lint::skills::frontmatter::ParseError;

    fn prompt(text: &str, line: usize) -> Prompt {
        Prompt {
            text: text.to_owned(),
            line,
        }
    }

    #[test]
    fn positives_and_near_misses_grouped_by_owner_are_read_with_their_lines() {
        let text = "# plan routing\npositives:\n  - break this spec into tasks\n  - \"order the work: by risk\"\n\nnear_misses:\n  spec:\n    - write the requirements\n  # comment\n  diagnose:\n    - 'why does it crash'\n";

        let fixture = parse(text);

        assert_eq!(
            fixture,
            Ok(Fixture {
                positives: vec![
                    prompt("break this spec into tasks", 3),
                    prompt("order the work: by risk", 4),
                ],
                near_misses: vec![
                    NearMiss {
                        owner: "spec".to_owned(),
                        prompt: prompt("write the requirements", 8),
                    },
                    NearMiss {
                        owner: "diagnose".to_owned(),
                        prompt: prompt("why does it crash", 11),
                    },
                ],
                positives_line: 2,
                near_misses_line: 6,
            }),
            "items keep file order and 1-based lines; comments and blanks skipped"
        );
    }

    #[test]
    fn malformed_fixtures_report_the_line_they_stop_at() {
        let cases = [
            ("skill: plan\n", 1),
            ("positives: x\n", 1),
            ("positives:\n- unindented\n", 2),
            ("positives:\n  -\n", 2),
            ("positives:\n  - [a, b]\n", 2),
            ("positives:\n  - {a: b}\n", 2),
            ("positives:\n  - a: b\n", 2),
            ("positives:\n\t- tabbed\n", 2),
            ("positives:\n  - a\n    - b\n", 3),
            ("positives:\n  - a\npositives:\n  - b\n", 3),
            ("near_misses:\n  - a\n", 2),
            ("near_misses:\n  spec:\n  - a\n", 3),
            ("near_misses:\n  spec: a\n", 2),
            ("near_misses:\n  spec:\n    - a\n  spec:\n    - b\n", 4),
            ("near_misses:\n  Spec Owner:\n    - a\n", 2),
            ("  positives:\n", 1),
        ];
        for (text, line) in cases {
            let result = parse(text);
            assert!(
                matches!(result, Err(ParseError { line: l, .. }) if l == line),
                "{text:?} should fail at line {line}: {result:?}"
            );
        }
    }

    #[test]
    fn the_readme_example_reads_as_documented() {
        let readme = include_str!("../../../../../evals/README.md");
        let example = readme
            .split("```yaml\n")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("evals/README.md has a yaml example");

        let fixture = parse(example).expect("the documented example parses");

        let owners: Vec<&str> = fixture
            .near_misses
            .iter()
            .map(|n| n.owner.as_str())
            .collect();
        assert_eq!(
            (fixture.positives.len(), owners),
            (2, vec!["spec", "diagnose"]),
            "two positives, one near miss per owner"
        );
        assert_eq!(
            fixture.positives.get(1).map(|p| p.text.as_str()),
            Some("estimate the work: which tasks come first"),
            "the quoted prompt keeps its colon"
        );
    }

    #[test]
    fn an_empty_file_has_no_prompts() {
        assert_eq!(
            parse("# nothing yet\n"),
            Ok(Fixture::default()),
            "comments only"
        );
    }
}
