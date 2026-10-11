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
//! quoted; a plain item may not contain `: ` or start with a YAML indicator). Every owner lists
//! at least one prompt. Anything YAML would read differently is a [`ParseError`] with the line
//! it stopped at: flow collections (`[`, `{`), tabs, double-quoted escapes other than `\"` and
//! `\\`, and plain values YAML types (null, `~`, booleans including `yes`/`no`/`on`/`off`,
//! numbers, dates). So a file this reader accepts reads as the same strings under a real YAML
//! parser (Tier 2, #40).

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

/// An `owner:` key under `near_misses:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Owner {
    /// The owner skill's name.
    pub(crate) name: String,
    /// 1-based line of the key.
    pub(crate) line: usize,
}

/// A parsed routing fixture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Fixture {
    /// Prompts that must rank the fixture's skill first.
    pub(crate) positives: Vec<Prompt>,
    /// Prompts whose owner must outrank the fixture's skill, in file order.
    pub(crate) near_misses: Vec<NearMiss>,
    /// Every near-miss owner key, in file order; each lists at least one prompt.
    pub(crate) owners: Vec<Owner>,
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
    reader.close_owner()?;
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
    /// How many prompts the current owner has listed so far.
    owner_prompts: usize,
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
        self.close_owner()?;
        self.owner = None;
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
        self.owner_prompts = self.owner_prompts.saturating_add(1);
        Ok(())
    }

    fn owner_key(&mut self, content: &str, line: usize) -> Result<(), ParseError> {
        let owner = content
            .strip_suffix(':')
            .filter(|name| is_skill_name(name))
            .ok_or_else(|| error(line, "expected `<owner-skill>:` followed by its prompts"))?;
        self.close_owner()?;
        if !self.owners.insert(owner.to_owned()) {
            return Err(error(line, "duplicate owner"));
        }
        self.fixture.owners.push(Owner {
            name: owner.to_owned(),
            line,
        });
        self.owner = Some(owner.to_owned());
        self.owner_prompts = 0;
        self.item_indent = None;
        Ok(())
    }

    /// Ends the current owner's list: an owner that listed no prompts is an error at its key.
    fn close_owner(&self) -> Result<(), ParseError> {
        match (&self.owner, self.fixture.owners.last()) {
            (Some(_), Some(owner)) if self.owner_prompts == 0 => {
                Err(error(owner.line, "owner lists no prompts"))
            }
            _ => Ok(()),
        }
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
        if scalar.starts_with('"') && has_unsupported_escape(scalar) {
            return Err(error(
                line,
                "only the \\\" and \\\\ escapes are supported; use single quotes",
            ));
        }
        let text = unquote(scalar, line)?;
        if text.trim().is_empty() {
            return Err(error(line, "empty prompt"));
        }
        if !scalar.starts_with(['"', '\'']) && is_typed_plain(&text) {
            return Err(error(
                line,
                "YAML reads this plain value as a null, boolean, number or date; quote it",
            ));
        }
        Ok(text)
    }
}

/// True when a double-quoted scalar holds a backslash escape other than `\"` or `\\`: YAML
/// would decode it (`\t`, `\n`, `é`) where this reader keeps it literal.
fn has_unsupported_escape(scalar: &str) -> bool {
    let mut chars = scalar.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && !matches!(chars.next(), Some('"' | '\\')) {
            return true;
        }
    }
    false
}

/// Plain values a YAML 1.1 or 1.2 resolver types instead of reading as a string: null,
/// booleans (including `yes`/`no`/`on`/`off`), numbers in any base or with `_` or `:`
/// separators, infinities, NaN, and dates or timestamps.
fn is_typed_plain(value: &str) -> bool {
    const WORDS: [&str; 14] = [
        "~", "null", "true", "false", "yes", "no", "on", "off", "y", "n", ".inf", "+.inf", "-.inf",
        ".nan",
    ];
    let lower = value.to_ascii_lowercase();
    let numeric = value.bytes().any(|b| b.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-._:eE".contains(&b));
    let based = ["0x", "0o", "0b"].iter().any(|prefix| {
        lower.strip_prefix(prefix).is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    });
    let dated = matches!(
        value.as_bytes(),
        [a, b, c, d, b'-', ..] if [a, b, c, d].iter().all(|x| x.is_ascii_digit())
    );
    WORDS.contains(&lower.as_str()) || numeric || based || dated
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
    use super::{Fixture, NearMiss, Owner, Prompt, parse};
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
                owners: vec![
                    Owner {
                        name: "spec".to_owned(),
                        line: 7,
                    },
                    Owner {
                        name: "diagnose".to_owned(),
                        line: 10,
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
            ("near_misses:\n  ghost:\n  spec:\n    - a\n", 2),
            ("near_misses:\n  spec:\n    - a\n  ghost:\n", 4),
            ("near_misses:\n  ghost:\npositives:\n  - a\n", 2),
            ("positives:\n  - a\n  - \"tab\\there\"\n", 3),
            ("positives:\n  - \"caf\\u00e9\"\n", 2),
            ("positives:\n  - ~\n", 2),
            ("positives:\n  - null\n", 2),
            ("positives:\n  - True\n", 2),
            ("positives:\n  - false\n", 2),
            ("positives:\n  - yes\n", 2),
            ("positives:\n  - Off\n", 2),
            ("positives:\n  - 42\n", 2),
            ("positives:\n  - -3.5e2\n", 2),
            ("positives:\n  - 1_000\n", 2),
            ("positives:\n  - 0x1F\n", 2),
            ("positives:\n  - .inf\n", 2),
            ("positives:\n  - 10:30\n", 2),
            ("positives:\n  - 2026-10-11\n", 2),
            ("positives:\n  - 2026-10-11T09:00:00Z\n", 2),
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
    fn prompts_that_only_start_like_typed_values_stay_text() {
        let text = "positives:\n  - 3 tasks remain\n  - null pointer crash\n  - true north\n  - \"42\"\n  - 'yes'\n  - \"say \\\"hi\\\" \\\\ bye\"\n";

        let prompts: Vec<String> = parse(text)
            .expect("all are strings in YAML too")
            .positives
            .into_iter()
            .map(|p| p.text)
            .collect();

        assert_eq!(
            prompts,
            [
                "3 tasks remain",
                "null pointer crash",
                "true north",
                "42",
                "yes",
                "say \"hi\" \\ bye"
            ],
            "quoted values and longer phrases are text"
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
