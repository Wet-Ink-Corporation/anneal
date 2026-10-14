//! `watch deploy`: a deployment's state through a pluggable [`DeployProvider`].
//!
//! The first provider is [`GitHubDeployments`], which reads the latest status of a GitHub
//! deployment (`GET repos/{owner}/{repo}/deployments/{id}/statuses?per_page=1`, newest first):
//! `success` is clear; `queued`, `pending`, `in_progress`, or no status yet, is pending; any
//! other state (`failure`, `error`, `inactive`, or one GitHub adds later) blocks, as a failed
//! check does (exit 4). `inactive` blocks because the deployment watched is no longer the one
//! live. Another provider implements [`DeployProvider`] and gets its own `--provider` value.

use serde::Deserialize;

use super::WatchError;
use super::github::{RepoRef, non_empty};
use super::poll::Source;
use super::snapshot::{Blocker, Pending, Snapshot};
use crate::dag::tracker::github::{GhApi, GhError};

/// Where a deployment stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeployState {
    /// It finished and is live.
    Succeeded,
    /// It has not finished; `state` is the provider's word for where it is (`none` before any).
    Pending {
        /// The provider's state.
        state: String,
    },
    /// It ended without succeeding.
    Failed {
        /// The provider's state.
        state: String,
        /// Its log or target page.
        url: Option<String>,
    },
}

/// A source of one deployment's state.
pub(crate) trait DeployProvider {
    /// The deployment's state now.
    ///
    /// # Errors
    /// The [`WatchError`] when the state cannot be read; [`WatchError::Missing`] when there is
    /// no such deployment.
    fn state(&self) -> Result<DeployState, WatchError>;
}

/// One GitHub deployment, read through `gh api`.
#[derive(Debug)]
pub(crate) struct GitHubDeployments<'a, G> {
    /// The REST port.
    pub(crate) gh: &'a G,
    /// The repository.
    pub(crate) repo: RepoRef<'a>,
    /// The deployment id.
    pub(crate) id: u64,
}

impl<G: GhApi> DeployProvider for GitHubDeployments<'_, G> {
    fn state(&self) -> Result<DeployState, WatchError> {
        let endpoint = format!(
            "repos/{}/deployments/{}/statuses?per_page=1",
            self.repo.rest_path(),
            self.id
        );
        let body = match self.gh.get(&endpoint) {
            Ok(body) => body,
            Err(GhError::NotFound { .. }) => {
                return Err(WatchError::Missing {
                    what: format!("deployment {}", self.id),
                });
            }
            Err(error) => return Err(error.into()),
        };
        let statuses: Vec<Status> =
            serde_json::from_slice(&body).map_err(|source| WatchError::Json {
                what: "the deployment's statuses",
                source,
            })?;
        Ok(match statuses.into_iter().next() {
            None => DeployState::Pending {
                state: "none".to_owned(),
            },
            Some(status) => {
                let url = non_empty(status.log_url).or_else(|| non_empty(status.target_url));
                judge_state(status.state, url)
            }
        })
    }
}

/// What a GitHub deployment `state` means, with its page `url`.
pub(super) fn judge_state(state: String, url: Option<String>) -> DeployState {
    match state.as_str() {
        "success" => DeployState::Succeeded,
        "queued" | "pending" | "in_progress" => DeployState::Pending { state },
        _ => DeployState::Failed { state, url },
    }
}

/// The fields of a deployment status read; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Status {
    state: String,
    #[serde(default)]
    log_url: Option<String>,
    #[serde(default)]
    target_url: Option<String>,
}

/// A deployment watched through `provider`, named `id` in the envelope.
#[derive(Debug)]
pub(crate) struct DeployWatch<'a, P> {
    /// Where its state comes from.
    pub(crate) provider: &'a P,
    /// The deployment id as given.
    pub(crate) id: &'a str,
}

impl<P: DeployProvider> Source for DeployWatch<'_, P> {
    fn poll(&self) -> Result<Snapshot, WatchError> {
        let mut snapshot = Snapshot::default();
        let id = self.id.to_owned();
        match self.provider.state()? {
            DeployState::Succeeded => {}
            DeployState::Pending { state } => {
                snapshot.pending.push(Pending::Deployment { id, state });
            }
            DeployState::Failed { state, url } => {
                snapshot
                    .blockers
                    .push(Blocker::Deployment { id, state, url });
            }
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests;
