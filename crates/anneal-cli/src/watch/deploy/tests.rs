//! The deployment watch and its GitHub Deployments provider against recorded REST answers.

use super::{DeployState, DeployWatch, GitHubDeployments};
use crate::watch::WatchError;
use crate::watch::fake::FakeGh;
use crate::watch::github::RepoRef;
use crate::watch::poll::Source;
use crate::watch::snapshot::{Blocker, Pending, Snapshot};

fn poll(fixture: &'static str) -> Result<Snapshot, WatchError> {
    let gh = FakeGh::new().route("/deployments/42", &[fixture]);
    let provider = GitHubDeployments {
        gh: &gh,
        repo: RepoRef::Current,
        id: 42,
    };
    DeployWatch {
        provider: &provider,
        id: "42",
    }
    .poll()
}

#[test]
fn the_provider_reads_the_latest_status_of_the_deployment() {
    let gh = FakeGh::new().route("/deployments/42", &["deploy-success.json"]);
    let repo = "octo/app".parse().expect("valid");
    let provider = GitHubDeployments {
        gh: &gh,
        repo: RepoRef::Named(&repo),
        id: 42,
    };

    let state = super::DeployProvider::state(&provider).expect("the fixture reads");

    assert_eq!(state, DeployState::Succeeded);
    assert_eq!(
        *gh.calls.borrow(),
        vec!["repos/octo/app/deployments/42/statuses?per_page=1".to_owned()],
        "one GET, newest status first"
    );
}

#[test]
fn a_successful_deployment_is_clear() {
    let snapshot = poll("deploy-success.json").expect("the fixture reads");

    assert_eq!(snapshot, Snapshot::default());
}

#[test]
fn a_failed_deployment_blocks_with_its_log() {
    let snapshot = poll("deploy-failure.json").expect("the fixture reads");

    assert_eq!(
        snapshot.blockers,
        vec![Blocker::Deployment {
            id: "42".to_owned(),
            state: "failure".to_owned(),
            url: Some("https://github.com/octo/app/actions/runs/702".to_owned()),
        }]
    );
}

#[test]
fn a_deployment_in_progress_or_with_no_status_yet_is_pending() {
    let running = poll("deploy-in-progress.json").expect("the fixture reads");
    let unstarted = poll("deploy-none.json").expect("the fixture reads");

    assert_eq!(
        (running.pending, unstarted.pending),
        (
            vec![Pending::Deployment {
                id: "42".to_owned(),
                state: "in_progress".to_owned()
            }],
            vec![Pending::Deployment {
                id: "42".to_owned(),
                state: "none".to_owned()
            }],
        )
    );
}

#[test]
fn an_unknown_deployment_is_a_query_error() {
    let gh = FakeGh::new().missing("/deployments/9");
    let provider = GitHubDeployments {
        gh: &gh,
        repo: RepoRef::Current,
        id: 9,
    };

    let result = DeployWatch {
        provider: &provider,
        id: "9",
    }
    .poll();

    assert!(
        matches!(&result, Err(WatchError::Missing { what }) if what == "deployment 9"),
        "{result:?}"
    );
}

#[test]
fn states_other_than_success_and_the_unfinished_ones_block() {
    let states = ["error", "failure", "inactive", "destroyed"]
        .map(|state| super::judge_state(state.to_owned(), None));

    assert!(
        states
            .iter()
            .all(|state| matches!(state, DeployState::Failed { .. })),
        "{states:?}"
    );
}
