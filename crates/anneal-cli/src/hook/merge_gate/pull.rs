//! What the gate needs of one `gh pr merge` before it reads any verdict: a pull request number,
//! the judged repository named by `-R`/`--repo <host>/<owner>/<repo>` (so neither the working
//! directory nor `GH_REPO`/`GH_HOST` chooses it), no `GH_REPO`/`GH_HOST` set and no directory
//! change before gh (defence in depth), a URL of no other repository, and a head pinned by a
//! full `--match-head-commit`.

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
    if let Some(repo) = merge.repo.as_deref().filter(|repo| !is_full_name(repo)) {
        return Err(Denial::new(
            Check::Repo,
            &format!(
                "gh pr merge --repo {repo} does not name a repository as HOST/OWNER/REPO, so \
                 GH_HOST or gh's reading of it could choose another; name this repository's \
                 origin: --repo <host>/<owner>/<repo>"
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

/// The full head commit `pull` pins, or the denial naming the exact command to run (with
/// `--repo` naming `origin` when it reads): the pin is what makes gh refuse to merge a head
/// other than the one judged.
pub(super) fn pinned_head<'a>(
    pull: Pull<'a>,
    origin: Option<&str>,
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
    let repo = origin.map_or_else(String::new, |origin| format!(" --repo {origin}"));
    Err(Denial::new(
        Check::Head,
        &format!(
            "gh pr merge {} has {given}, so gh could merge a head the gate did not judge; pin \
             it: `gh pr merge {} --match-head-commit {head}{repo}`",
            pull.pr, pull.pr
        ),
    ))
}

/// Whether `repo` is `<host>/<owner>/<repo>` in the characters host names and GitHub
/// repository names use, which leaves out every expansion, glob and quote: gh then takes the
/// host from it, not from `GH_HOST`.
fn is_full_name(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(host), Some(owner), Some(name), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    without_git_suffix(name).is_none() && full_name(host, owner, name).is_some()
}

/// `name` without a `.git` ending in any case, or `None` when it has none.
fn without_git_suffix(name: &str) -> Option<&str> {
    name.rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("git"))
        .map(|(stem, _)| stem)
}

/// The one rule both `--repo` and `origin` are read with: `<host>/<owner>/<repo>`, lower-cased,
/// each part non-empty and in the characters host names and GitHub repository names use.
fn full_name(host: &str, owner: &str, name: &str) -> Option<String> {
    let name_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    [host, owner, name]
        .iter()
        .all(|part| !part.is_empty() && part.chars().all(name_char))
        .then(|| format!("{host}/{owner}/{name}").to_ascii_lowercase())
}

/// The `<host>/<owner>/<repo>` of the `origin` remote under `root`: `None` when there is no
/// `origin` or its URL does not read as one, or the denial when git cannot be run.
pub(super) fn read_origin(root: &Path, timeout: Duration) -> Result<Option<String>, Denial> {
    let args = ["remote", "get-url", "origin"];
    match paths::run_git_capped(root, &args, timeout, REMOTE_OUTPUT_BYTES) {
        Ok(output) if output.status.success() => {
            Ok(origin_repo(&String::from_utf8_lossy(&output.stdout)))
        }
        // git ran and found no `origin`.
        Ok(_) => Ok(None),
        Err(error) => Err(unjudged("the origin remote cannot be read", &error)),
    }
}

/// `None` when `url_repo`, the repository a pull request URL named, is `origin`'s; else the
/// denial. A repository with no `origin`, or one whose URL does not read, is denied too.
pub(super) fn check_url(url_repo: &str, origin: Option<&str>) -> Option<Denial> {
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

/// `None` when `repo`, the `--repo` of a merge of `pr` pinned to `head`, names `origin`, the
/// repository the gate judges; else the denial naming the exact command. gh reads the
/// repository from `--repo` before `GH_REPO` and the working directory, and the host in it
/// before `GH_HOST`, so this pins what gh merges in. With no readable `origin` the gate cannot
/// tell which repository it judges, so it denies.
pub(super) fn check_repo(
    pr: u64,
    head: &str,
    repo: Option<&str>,
    origin: Option<&str>,
) -> Option<Denial> {
    let Some(origin) = origin else {
        return Some(Denial::new(
            Check::Repo,
            "this repository's origin remote cannot be read as <host>/<owner>/<repo>, so the \
             gate cannot tell which repository gh pr merge --repo must name; add the origin \
             remote of the pull request's repository",
        ));
    };
    let given = match repo {
        Some(repo) if repo.eq_ignore_ascii_case(origin) => return None,
        Some(repo) => format!("names {repo}, not this repository's origin ({origin})"),
        None => {
            "names no --repo, so the working directory or GH_REPO chooses the repository".to_owned()
        }
    };
    Some(Denial::new(
        Check::Repo,
        &format!(
            "gh pr merge {pr} {given}; name the repository the gate judged: \
             `gh pr merge {pr} --match-head-commit {head} --repo {origin}`"
        ),
    ))
}

/// The `<host>/<owner>/<repo>` (no port, user or `.git` in any case; read by [`full_name`]'s
/// rule, as `--repo` is) of a remote URL:
/// `scheme://[user@]host[:port]/owner/repo[.git]` or scp-like `[user@]host:owner/repo[.git]`.
pub(crate) fn origin_repo(url: &str) -> Option<String> {
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
    let (owner, repo) = path.trim_matches('/').split_once('/')?;
    full_name(host, owner, without_git_suffix(repo).unwrap_or(repo))
}
