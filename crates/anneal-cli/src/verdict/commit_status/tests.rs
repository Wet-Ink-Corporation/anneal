use std::ffi::OsString;

use super::{
    API_VAR, GITHUB_API, GithubEnv, GithubRepo, PostError, State, Token, description, state_of,
};
use crate::hook::merge_gate::Check;
use crate::verdict::{Level, Source};

/// GitHub's limit on a commit status description, in characters.
const MAX_DESCRIPTION_CHARS: usize = 140;

/// A lookup over `pairs`, as the binary's environment would answer.
fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let pairs: Vec<(String, OsString)> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
        .collect();
    move |var: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == var)
            .map(|(_, value)| value.clone())
    }
}

#[test]
fn github_com_origins_read_as_owner_and_repository_in_every_url_form() {
    for url in [
        "https://github.com/Owner/Repo.git",
        "https://github.com/owner/repo",
        "https://user@github.com/owner/repo.git\n",
        "git@github.com:owner/repo.git",
        "git@github.com:owner/repo",
        "ssh://git@github.com/owner/repo.git",
        "ssh://git@github.com:22/owner/repo.git",
    ] {
        let repo = GithubRepo::from_remote_url(url);
        assert_eq!(
            repo.map(|repo| repo.to_string()).ok(),
            Some("owner/repo".to_owned()),
            "{url:?}"
        );
    }
}

#[test]
fn other_hosts_and_shapes_are_not_github_com_repositories() {
    for url in [
        "https://gitlab.com/owner/repo.git",
        "git@github-work:owner/repo.git",
        "https://github.example.com/owner/repo",
    ] {
        let refused = GithubRepo::from_remote_url(url);
        assert!(
            matches!(refused, Err(PostError::NotGithub { .. })),
            "{url:?}: {refused:?}"
        );
    }
    for url in ["/srv/git/repo.git", "https://github.com/owner", ""] {
        let refused = GithubRepo::from_remote_url(url);
        assert!(
            matches!(refused, Err(PostError::OriginUnreadable)),
            "{url:?}: {refused:?}"
        );
    }
}

#[test]
fn a_not_github_origin_is_named_without_its_user_or_credentials() {
    let refused = GithubRepo::from_remote_url("https://alice:s3cret@gitlab.com/owner/repo.git");

    let message = refused
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("gitlab.com/owner/repo"), "{message}");
    assert!(
        !message.contains("s3cret") && !message.contains("alice"),
        "{message}"
    );
}

#[test]
fn gh_token_is_read_before_github_token_and_an_empty_one_is_skipped() {
    let both = GithubEnv::read(lookup(&[("GH_TOKEN", "gh1"), ("GITHUB_TOKEN", "gh2")]));
    let empty_first = GithubEnv::read(lookup(&[("GH_TOKEN", ""), ("GITHUB_TOKEN", "gh2")]));
    let none = GithubEnv::read(lookup(&[("GH_TOKEN", "")]));

    let secret = |env: &GithubEnv| match &env.token {
        Some(Token::Valid(token)) => Some(token.expose_secret().to_owned()),
        Some(Token::Unfit { .. }) | None => None,
    };
    assert_eq!(secret(&both), Some("gh1".to_owned()));
    assert_eq!(secret(&empty_first), Some("gh2".to_owned()));
    assert!(none.token.is_none(), "an empty token is no token");
}

#[test]
fn a_token_unfit_for_a_header_is_kept_as_unfit_naming_its_variable() {
    let env = GithubEnv::read(lookup(&[("GITHUB_TOKEN", "two words")]));

    assert!(
        matches!(
            env.token,
            Some(Token::Unfit {
                var: "GITHUB_TOKEN"
            })
        ),
        "{env:?}"
    );
    assert!(!format!("{env:?}").contains("two words"), "{env:?}");
}

#[test]
fn the_api_override_is_honoured_only_in_debug_builds_and_only_on_loopback() {
    let local = GithubEnv::read(lookup(&[(API_VAR, "http://127.0.0.1:8080/api/")]));
    let remote = GithubEnv::read(lookup(&[(API_VAR, "https://evil.example/")]));
    let unset = GithubEnv::read(lookup(&[]));

    assert_eq!(
        local.api_base(true).ok(),
        Some("http://127.0.0.1:8080/api".to_owned())
    );
    assert_eq!(local.api_base(false).ok(), Some(GITHUB_API.to_owned()));
    assert!(matches!(remote.api_base(true), Err(PostError::Endpoint)));
    assert_eq!(remote.api_base(false).ok(), Some(GITHUB_API.to_owned()));
    assert_eq!(unset.api_base(true).ok(), Some(GITHUB_API.to_owned()));
}

#[test]
fn the_merge_gate_s_answer_maps_to_a_status_state() {
    assert_eq!(state_of(None), State::Success, "the gate allows the merge");
    for check in [Check::NoVerdict, Check::Stale, Check::Tests, Check::Review] {
        assert_eq!(
            state_of(Some(check)),
            State::Pending,
            "{check:?}: a later verdict can still satisfy the gate"
        );
    }
    for check in [Check::ChangesRequested, Check::Unjudged, Check::Shape] {
        assert_eq!(state_of(Some(check)), State::Failure, "{check:?}");
    }
}

#[test]
fn the_description_names_the_level_the_source_and_the_gate_s_answer() {
    let cases = [
        (
            Level::TestsVerified,
            Source::Local,
            None,
            "tests-verified by local: the merge gate passes",
        ),
        (
            Level::TestsVerified,
            Source::ReviewPlugin,
            Some(Check::Review),
            "tests-verified by review-plugin: the merge gate wants review",
        ),
        (
            Level::Approved,
            Source::Human,
            Some(Check::Unjudged),
            "approved by human: the merge gate denies (unjudged)",
        ),
    ];
    for (level, source, check, expected) in cases {
        assert_eq!(description(level, source, check), expected);
    }
}

#[test]
fn every_description_fits_github_s_limit() {
    let checks = [
        None,
        Some(Check::ChangesRequested),
        Some(Check::NoVerdict),
        Some(Check::Unjudged),
    ];
    for level in [Level::TestsVerified, Level::Reviewed, Level::Approved] {
        for source in [Source::Local, Source::ReviewPlugin, Source::Human] {
            for check in checks {
                let text = description(level, source, check);
                assert!(text.chars().count() <= MAX_DESCRIPTION_CHARS, "{text}");
            }
        }
    }
}
