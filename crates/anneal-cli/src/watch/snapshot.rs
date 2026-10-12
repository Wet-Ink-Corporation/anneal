//! What one poll saw: the blockers, and the work still pending.
//!
//! A blocker settles the watch at once; pending work makes it poll again. The exit code of a
//! blocked watch is the lowest code among its blockers (conflict 2, threads 3, checks and
//! deployments 4, draft, merge state and the merge gate 6), and every blocker is listed.

use serde::Serialize;

use crate::envelope::Finding;
use crate::exit_codes::{self, Code};

/// One poll's view of the target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
    /// The commit the checks were read for, when the target has one.
    pub(crate) head: Option<String>,
    /// What stops the target, each named.
    pub(crate) blockers: Vec<Blocker>,
    /// What has not finished yet, each named.
    pub(crate) pending: Vec<Pending>,
}

impl Snapshot {
    /// True when polling again cannot change the answer: something blocks, or nothing is
    /// pending.
    pub(crate) fn settled(&self) -> bool {
        !self.blockers.is_empty() || self.pending.is_empty()
    }

    /// The lowest exit code among the blockers; `None` when nothing blocks.
    pub(crate) fn blocker_code(&self) -> Option<Code> {
        self.blockers
            .iter()
            .map(Blocker::code)
            .min_by_key(|code| code.value())
    }
}

/// Something that stops the target from being clear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Blocker {
    /// The pull request conflicts with its base.
    Conflict {
        /// The pull request number.
        pr: u64,
        /// Its web address.
        url: String,
    },
    /// An unresolved review thread.
    Thread {
        /// The thread's first comment.
        url: Option<String>,
        /// The file the thread is on.
        path: Option<String>,
    },
    /// A check run or commit status that did not pass.
    Check {
        /// The check's name (a check run's name, or a status's context).
        name: String,
        /// How it ended, in GitHub's words lower-cased (`failure`, `timed_out`, `error`).
        conclusion: String,
        /// Its details page.
        url: Option<String>,
    },
    /// A deployment whose latest status is not a success.
    Deployment {
        /// The deployment id.
        id: String,
        /// Its latest state (`failure`, `error`, `inactive`).
        state: String,
        /// Its log or target page.
        url: Option<String>,
    },
    /// The pull request is a draft, which GitHub does not merge.
    Draft {
        /// The pull request number.
        pr: u64,
        /// Its web address.
        url: String,
    },
    /// GitHub refuses the merge for a reason the checks and threads read do not show: branch
    /// protection still `blocked` once every check passed and every thread is resolved (a
    /// required review, or a required check that never reported), `behind`, or a state this
    /// version does not know.
    MergeState {
        /// The pull request number.
        pr: u64,
        /// GitHub's `mergeable_state`.
        state: String,
        /// Its web address.
        url: String,
    },
    /// The merge gate would deny a merge of the head.
    Gate {
        /// The gate's check id (`no-verdict`, `stale`, `review`, ...).
        check: &'static str,
        /// The gate's one-line reason, naming the command that clears it.
        reason: String,
    },
}

impl Blocker {
    /// The exit code this blocker means.
    pub(crate) const fn code(&self) -> Code {
        match self {
            Self::Conflict { .. } => exit_codes::WATCH_CONFLICT,
            Self::Thread { .. } => exit_codes::WATCH_THREADS,
            Self::Check { .. } | Self::Deployment { .. } => exit_codes::WATCH_CHECKS,
            Self::Draft { .. } | Self::MergeState { .. } | Self::Gate { .. } => {
                exit_codes::WATCH_GATE
            }
        }
    }

    /// The finding that names this blocker, with a stable key.
    pub(crate) fn finding(&self) -> Finding {
        match self {
            Self::Conflict { pr, url } => Finding::new(
                format!("watch/conflict:{pr}"),
                format!("pull request {pr} conflicts with its base ({url}); merge or rebase it"),
            ),
            Self::Thread { url, path } => {
                let at = url.as_deref().unwrap_or("a thread with no comment");
                let on = path
                    .as_deref()
                    .map_or_else(String::new, |path| format!(" on {path}"));
                Finding::new(
                    format!("watch/thread:{at}"),
                    format!("unresolved review thread{on}: {at}"),
                )
            }
            Self::Check {
                name,
                conclusion,
                url,
            } => Finding::new(
                format!("watch/check:{name}"),
                format!("check {name:?} ended {conclusion}{}", at(url.as_deref())),
            ),
            Self::Deployment { id, state, url } => Finding::new(
                format!("watch/deployment:{id}"),
                format!("deployment {id} is {state}{}", at(url.as_deref())),
            ),
            Self::Draft { pr, url } => Finding::new(
                format!("watch/draft:{pr}"),
                format!(
                    "pull request {pr} is a draft ({url}); mark it ready with gh pr ready {pr}"
                ),
            ),
            Self::MergeState { pr, state, url } => Finding::new(
                format!("watch/merge-state:{state}"),
                format!(
                    "GitHub will not merge pull request {pr} (mergeable_state {state}, {url}): \
                     a branch protection rule, such as a required review or check, is not met"
                ),
            ),
            Self::Gate { check, reason } => {
                Finding::new(format!("watch/gate:{check}"), reason.as_str())
            }
        }
    }
}

/// Work that has not finished, so the watch polls again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Pending {
    /// A check run or commit status still running or queued.
    Check {
        /// The check's name.
        name: String,
        /// Its state, lower-cased (`queued`, `in_progress`, `pending`, `expected`).
        status: String,
    },
    /// No check has reported on the commit yet.
    NoChecks,
    /// GitHub has not computed whether the pull request merges cleanly.
    Mergeable,
    /// GitHub does not yet allow the merge (`mergeable_state` `blocked` while checks still run,
    /// or `unstable`).
    MergeState {
        /// GitHub's `mergeable_state`.
        state: String,
    },
    /// A deployment not yet finished, or with no status yet.
    Deployment {
        /// The deployment id.
        id: String,
        /// Its latest state, or `none`.
        state: String,
    },
}

impl Pending {
    /// The finding that names this pending item when the watch times out.
    pub(crate) fn finding(&self) -> Finding {
        match self {
            Self::Check { name, status } => Finding::new(
                format!("watch/pending:check:{name}"),
                format!("check {name:?} is still {status}"),
            ),
            Self::NoChecks => Finding::new(
                "watch/pending:no-checks",
                "no check has reported on the commit yet",
            ),
            Self::Mergeable => Finding::new(
                "watch/pending:mergeable",
                "GitHub has not yet computed whether the pull request merges cleanly",
            ),
            Self::MergeState { state } => Finding::new(
                format!("watch/pending:merge-state:{state}"),
                format!("GitHub still reports the pull request {state}"),
            ),
            Self::Deployment { id, state } => Finding::new(
                format!("watch/pending:deployment:{id}"),
                format!("deployment {id} is still {state}"),
            ),
        }
    }
}

/// ` (<url>)`, or nothing.
fn at(url: Option<&str>) -> String {
    url.map_or_else(String::new, |url| format!(" ({url})"))
}

#[cfg(test)]
mod tests {
    use super::{Blocker, Pending, Snapshot};
    use crate::exit_codes::Code;

    fn check(name: &str) -> Blocker {
        Blocker::Check {
            name: name.to_owned(),
            conclusion: "failure".to_owned(),
            url: None,
        }
    }

    #[test]
    fn the_lowest_code_among_several_blockers_wins() {
        let snapshot = Snapshot {
            head: None,
            blockers: vec![
                Blocker::Gate {
                    check: "no-verdict",
                    reason: "merge-gate: none".to_owned(),
                },
                check("test"),
                Blocker::Thread {
                    url: None,
                    path: None,
                },
            ],
            pending: Vec::new(),
        };

        let code = snapshot.blocker_code().map(Code::value);

        assert_eq!(
            code,
            Some(3),
            "threads (3) before checks (4) and the gate (6)"
        );
    }

    #[test]
    fn a_snapshot_settles_on_a_blocker_or_when_nothing_is_pending() {
        let pending = Snapshot {
            pending: vec![Pending::NoChecks],
            ..Snapshot::default()
        };
        let blocked = Snapshot {
            blockers: vec![check("lint")],
            ..pending.clone()
        };

        let settled = [
            Snapshot::default().settled(),
            pending.settled(),
            blocked.settled(),
        ];

        assert_eq!(settled, [true, false, true], "clear, pending, blocked");
    }
}
