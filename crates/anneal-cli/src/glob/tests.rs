use super::{Glob, GlobError, MAX_GLOB_BYTES};

fn glob(pattern: &str) -> Glob {
    Glob::new(pattern).expect("a valid pattern")
}

#[test]
fn a_double_star_matches_any_number_of_directories_at_the_start_middle_and_end() {
    let start = glob("**/migrations/**");
    let middle = glob("src/**/auth.rs");
    let end = glob(".github/**");

    assert_eq!(
        [
            start.matches("migrations/0001.sql"),
            start.matches("db/schema/migrations/0001.sql"),
            start.matches("db/migration/0001.sql"),
            middle.matches("src/auth.rs"),
            middle.matches("src/a/b/auth.rs"),
            middle.matches("lib/auth.rs"),
            end.matches(".github/workflows/ci.yml"),
            end.matches("docs/.github/x"),
        ],
        [true, true, false, true, true, false, true, false],
        "`**` spans zero or more whole directories; the rest is anchored to the root"
    );
}

#[test]
fn a_star_stays_inside_one_path_segment_and_a_question_mark_is_one_character() {
    let star = glob("**/*auth*");
    let question = glob("v?.sql");

    assert_eq!(
        [
            star.matches("src/oauth_client.rs"),
            star.matches("authz"),
            star.matches("src/auth/mod.rs"),
            glob("src/*.rs").matches("src/a/b.rs"),
            question.matches("v1.sql"),
            question.matches("v12.sql"),
        ],
        [true, true, false, false, true, false],
        "`*` never crosses a `/`"
    );
}

#[test]
fn matching_ignores_ascii_letter_case() {
    assert!(
        glob("**/Dockerfile*").matches("deploy/DOCKERFILE.prod"),
        "case-insensitive, as protected paths are in the guard"
    );
}

#[test]
fn an_empty_absolute_or_oversized_pattern_is_refused() {
    let long = "a".repeat(MAX_GLOB_BYTES + 1);

    assert_eq!(
        [
            Glob::new("").err(),
            Glob::new("/etc/**").err(),
            Glob::new(&long).err(),
            Glob::new("a\\b").err(),
        ],
        [
            Some(GlobError::Empty),
            Some(GlobError::Absolute),
            Some(GlobError::TooLong),
            Some(GlobError::Backslash),
        ],
        "each refusal names its reason"
    );
}

#[test]
fn the_pattern_is_kept_as_written() {
    assert_eq!(glob("**/*.TF").as_str(), "**/*.TF", "for reports");
}
