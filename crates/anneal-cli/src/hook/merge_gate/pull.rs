//! What the gate needs of one `gh pr merge` before it reads any verdict: a pull request number,
//! no other repository (`-R`/`--repo`, `GH_REPO`/`GH_HOST`, a URL of another repository), and
//! no directory change before gh, and a head pinned by a full `--match-head-commit`.

use std::path::Path;
use std::time::Duration;

use super::{Check, Denial, unjudged};
use crate::hook::guard::{PrMerge, Target};
use crate::paths;
use crate::pr_diff::{self, is_object_id};

/// Most bytes kept from `git remote get-url`.
const REMOTE_OUTPUT_BYTES: u64 = 64 * 1024;

/// A merge whose pull request the gate can look up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pull<'a> {
    /// The pull request number.
    pub(super) pr: u64,
    /// `--match-head-commit` as given.
    pub(super) pin: Option<&'a str>,
    /// The `<host>/<owner>/<repo>` of the URL that named it, if one did.
    pub(super) url_repo: Option<&'a str>,
}

/// The pull request of `merge`, or why the gate refuses it without reading anything.
pub(super) fn pull(merge: &PrMerge) -> Result<Pull<'_>, Denial> {
    if let Some(repo) = &merge.repo {
        return Err(Denial::new(
            Check::Repo,
            &format!(
                "gh pr merge --repo {repo} merges in another repository, whose verdicts are \
                 not in this ledger; run it without --repo from that repository's worktree"
            ),
        ));
    }
    if let Some(name) = &merge.env {
        return Err(Denial::new(
            Check::Env,
            &format!(
                "the command sets {name}, which can point gh pr merge at another repository or \
                 host; run gh pr merge without setting {name}"
            ),
        ));
    }
    if let Some(how) = merge.moved {
        return Err(Denial::new(
            Check::Directory,
            &format!(
                "the command runs {how} before gh pr merge, so gh may read another repository; \
                 run gh pr merge from this worktree without changing directory first"
            ),
        ));
    }
    match merge.target {
        Target::Number(pr) => Ok(Pull {
            pr,
            pin: merge.match_head.as_deref(),
            url_repo: merge.url_repo.as_deref(),
        }),
        Target::Missing => Err(Denial::new(
            Check::Target,
            "gh pr merge with no pull request merges the current branch's, which the gate \
             cannot name; name it by number: gh pr merge <number>",
        )),
        Target::Unnumbered => Err(Denial::new(
            Check::Target,
            "gh pr merge names its pull request by branch, by a word known only at run time, \
             or by something other than an http(s)://<host>/<owner>/<repo>/pull/<number> URL; \
             name it by number: gh pr merge <number>",
        )),
    }
}

/// The full head commit `pull` pins, or the denial naming the exact command to run: the pin is
/// what makes gh refuse to merge a head other than the one judged.
pub(super) fn pinned_head<'a>(
    pull: Pull<'a>,
    root: &Path,
    timeout: Duration,
) -> Result<&'a str, Denial> {
    if let Some(pin) = pull.pin.filter(|pin| is_object_id(pin)) {
        return Ok(pin);
    }
    let head = match pr_diff::resolve_commit(root, "HEAD", timeout) {
        Ok(Some(head)) => head,
        // An unborn branch: the command is named with a placeholder.
        Ok(None) => "<the full commit id of the head you verified>".to_owned(),
        Err(error) => return Err(unjudged("HEAD cannot be resolved", &error)),
    };
    let given = pull.pin.map_or_else(
        || "no --match-head-commit".to_owned(),
        |pin| format!("--match-head-commit {pin}, not a full commit id"),
    );
    Err(Denial::new(
        Check::Head,
        &format!(
            "gh pr merge {} has {given}, so gh could merge a head the gate did not judge; pin \
             it: `gh pr merge {} --match-head-commit {head}`",
            pull.pr, pull.pr
        ),
    ))
}

/// `None` when `url_repo`, the repository a pull request URL named, is `origin`'s; else the
/// denial. A repository with no `origin`, or one whose URL does not read, is denied too.
pub(super) fn check_origin(url_repo: &str, root: &Path, timeout: Duration) -> Option<Denial> {
    let args = ["remote", "get-url", "origin"];
    let origin = match paths::run_git_capped(root, &args, timeout, REMOTE_OUTPUT_BYTES) {
        Ok(output) if output.status.success() => {
            origin_repo(&String::from_utf8_lossy(&output.stdout))
        }
        // git ran and found no `origin`: denied below.
        Ok(_) => None,
        Err(error) => return Some(unjudged("the origin remote cannot be read", &error)),
    };
    match origin {
        Some(origin) if origin == url_repo => None,
        Some(origin) => Some(Denial::new(
            Check::Repo,
            &format!(
                "the pull request URL names {url_repo}, not this repository's origin \
                 ({origin}); merge it from that repository's worktree"
            ),
        )),
        None => Some(Denial::new(
            Check::Repo,
            &format!(
                "the pull request URL names {url_repo}, and this repository's origin remote \
                 cannot be read to compare; name the pull request by number"
            ),
        )),
    }
}

/// The `<host>/<owner>/<repo>` (lower-cased, no port, user or `.git`) of a remote URL:
/// `scheme://[user@]host[:port]/owner/repo[.git]` or scp-like `[user@]host:owner/repo[.git]`.
pub(super) fn origin_repo(url: &str) -> Option<String> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => url
            .split_once(':')
            .filter(|(host, _)| !host.contains('/'))?,
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(name, _port)| name);
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if host.is_empty() || owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("{host}/{owner}/{repo}").to_ascii_lowercase())
}
