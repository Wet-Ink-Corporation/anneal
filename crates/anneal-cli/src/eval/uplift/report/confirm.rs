//! Watched guards and the command that confirms one: the case alone, the with arm only, 10
//! runs.

use std::borrow::Cow;

use serde::Serialize;

use crate::envelope::Finding;
use crate::eval::uplift::runs::Runs;
use crate::eval::uplift::suites::Case;
use crate::eval::uplift::tally::{CaseResult, GuardBand};

/// A guard on watch, as `data.watch` lists it.
#[derive(Debug, Serialize)]
pub(super) struct Watch<'a> {
    /// Its skill.
    pub(super) skill: &'a str,
    /// Its case file, relative to the suites directory.
    pub(super) case: &'a str,
    /// Its passing runs with the skill.
    passed: u32,
    /// The runs asked for.
    runs: u32,
    /// The command that confirms it.
    confirm: String,
}

/// Each guard of `results` on watch, with the command that confirms it, given the run's
/// `cases`, the `runs` asked for and the words its confirmation carries over (`prefix`).
pub(super) fn watched<'a>(
    results: &[CaseResult<'a>],
    cases: &[Case],
    runs: Runs,
    prefix: &[String],
) -> Vec<Watch<'a>> {
    results
        .iter()
        .filter(|result| result.guard_band == Some(GuardBand::Watch))
        .filter_map(|result| {
            // Every result is of one of the run's cases, so its file names one.
            let case = cases.iter().find(|case| case.file == result.file)?;
            Some(Watch {
                skill: result.skill,
                case: result.file,
                passed: result.with.passed,
                runs: runs.get(),
                confirm: command(prefix, case),
            })
        })
        .collect()
}

/// The finding of a guard on watch.
pub(super) fn watch_finding(watch: &Watch<'_>) -> Finding {
    Finding::new(
        format!("eval/uplift/watch:{}", watch.case),
        format!(
            "guard case {} passed {} of {} runs with the skill: on watch, not failed; confirm \
             it (it holds at 9 of 10): {}",
            watch.case, watch.passed, watch.runs, watch.confirm
        ),
    )
}

/// `anneal eval uplift <prefix…> --case <file> --no-baseline --runs 10`, then
/// `--allow-gated-tools` when `case` names a gated tool and `--allow-scaffolds` when it names a
/// fixture, each word quoted for a POSIX shell. `prefix` is the flags carried over from the
/// round ([`crate::eval::uplift::UpliftArgs::confirm_words`]).
pub(super) fn command(prefix: &[String], case: &Case) -> String {
    let runs = Runs::CONFIRMATION.get().to_string();
    let gated = case
        .body
        .allowed_tools
        .iter()
        .any(|tool| tool.is_gated())
        .then_some("--allow-gated-tools");
    let scaffolded = case.fixture.is_some().then_some("--allow-scaffolds");
    let words = ["anneal", "eval", "uplift"]
        .into_iter()
        .chain(prefix.iter().map(String::as_str))
        .chain(["--case", case.file.as_str(), "--no-baseline", "--runs"])
        .chain([runs.as_str()])
        .chain(gated)
        .chain(scaffolded);
    words.map(shell_word).collect::<Vec<_>>().join(" ")
}

/// `word` as one POSIX shell word: as it is when every byte is one a shell leaves alone, else
/// single-quoted, each `'` in it written `'\''`.
fn shell_word(word: &str) -> Cow<'_, str> {
    let plain = !word.is_empty()
        && word
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./=:,+@%-".contains(&byte));
    if plain {
        Cow::Borrowed(word)
    } else {
        Cow::Owned(format!("'{}'", word.replace('\'', r"'\''")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{command, shell_word};
    use crate::eval::uplift::case::parse;
    use crate::eval::uplift::fixture::Fixture;
    use crate::eval::uplift::suites::{Case, SuiteKind};

    /// A `build` regression guard named `slug`, naming `tools`.
    fn guard(slug: &str, tools: &[&str]) -> Case {
        let text = json!({
            "behaviour": "guard",
            "seed": "s",
            "prompt": "p",
            "allowed_tools": tools,
            "graders": [{"type": "regex", "name": "a", "pattern": "x"}]
        })
        .to_string();
        Case {
            skill: "build".to_owned(),
            suite: SuiteKind::Regression,
            slug: slug.to_owned(),
            file: format!("build/regression/{slug}.json"),
            body: parse(text.as_bytes()).expect("valid"),
            fixture: None,
        }
    }

    #[test]
    fn a_gated_fixture_guard_is_confirmed_alone_without_a_baseline_at_10_runs_with_its_opt_ins() {
        let case = Case {
            fixture: Some(Fixture::sample("py-app")),
            ..guard("plan-before-source", &["Skill", "Read", "Write", "Bash"])
        };

        assert_eq!(
            command(&[], &case),
            "anneal eval uplift --case build/regression/plan-before-source.json --no-baseline \
             --runs 10 --allow-gated-tools --allow-scaffolds"
        );
    }

    #[test]
    fn a_guard_with_no_gated_tool_or_fixture_needs_no_opt_in_and_a_spaced_work_dir_is_quoted() {
        let prefix = ["--work-dir".to_owned(), "/srv/it's here".to_owned()];

        assert_eq!(
            command(&prefix, &guard("g", &["Skill", "Read"])),
            r"anneal eval uplift --work-dir '/srv/it'\''s here' --case build/regression/g.json --no-baseline --runs 10"
        );
    }

    #[test]
    fn an_empty_word_is_quoted_so_it_stays_a_word() {
        assert_eq!(shell_word(""), "''");
    }
}
