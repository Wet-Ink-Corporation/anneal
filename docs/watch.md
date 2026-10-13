# Watching a pull request, CI or a deployment

`anneal watch` long-polls one target until it settles or a timeout passes, then exits with
one code that says what is blocking (#34). The agent runs it once instead of polling `gh`
itself, so it spends no context on "still running".

```
anneal watch pr <n>                                   [--timeout <secs>]
anneal watch ci <ref>         [--repo <owner/name>]   [--timeout <secs>]
anneal watch deploy <id>      [--provider github] [--repo <owner/name>] [--timeout <secs>]
```

- `pr <n>`: a pull request of gh's current repository (its remotes, or `GH_REPO`). It has no
  `--repo`: it ends with the merge gate, which reads this worktree's ledger.
- `ci <ref>`: the checks of the commit a branch, tag or commit names.
- `deploy <id>`: the latest status of a deployment. `--provider` picks where it is read; the
  first and default provider is `github` (GitHub Deployments), whose ids are numbers
  (`gh api repos/{owner}/{repo}/deployments` lists them).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | clear: nothing blocks and nothing is pending (for `pr`, the merge gate also allows the head) |
| 2 | merge conflict |
| 3 | unresolved review threads |
| 4 | failing checks, or a failed deployment |
| 5 | timeout: work was still pending when `--timeout` passed |
| 6 | gate: the pull request is a draft, GitHub's branch protection refuses the merge (`mergeable_state`), or the merge gate would deny merging the head |
| 7 | query error: the target could not be read; one `ERROR` line, nothing on stdout |

With several blockers the lowest code wins (2, then 3, then 4, then 6) and every blocker is
listed. A usage error also exits 2, but writes nothing on stdout, while a conflict always comes
with its envelope.

## Polling and the timeout

Each poll reads the target once. A blocker ends the watch at once, even while other checks are
still running, so the agent can act on a failure without waiting for the rest. When nothing
blocks and something is pending, the watch sleeps and polls again: 10 s after the first poll,
then 20 s, 40 s, and 60 s from then on, never past the deadline. The default timeout is
**1800 s (30 minutes)**; `--timeout 0` polls once and exits 5 if anything is still pending.
The deadline is checked between polls, never inside one: a poll that has started runs to its
end, so a watch can end up to one poll's time after `--timeout`. A poll's calls run one after
another, each `gh` call bounded at 30 s: up to 6 calls for `pr` (180 s, when GraphQL is refused
and the proxy route is read, and a `blocked` pull request is read again), 3 for `ci` (90 s)
and 1 for `deploy` (30 s). For `pr`, the merge gate's git calls follow, each bounded at 10 s,
with no overall limit on them.

## What blocks, and what is pending

| Seen | Result |
|---|---|
| a check run not `completed` (`queued`, `in_progress`, `waiting`, ...) | pending |
| a check run `completed` with `success`, `neutral` or `skipped` | passes |
| a check run `completed` with any other conclusion (`failure`, `timed_out`, `cancelled`, `action_required`, `stale`, ...) | blocks (4) |
| a commit status `pending` | pending |
| a commit status `success` | passes |
| a commit status `failure` or `error` | blocks (4) |
| no check run and no status at all | pending: no evidence is not green |
| `pr`: GitHub has not computed mergeability (`mergeable: null`, or `mergeable_state` `unknown`) | pending |
| `pr`: `mergeable: false`, or `mergeable_state` `dirty` | blocks (2) |
| `pr`: a draft (`draft: true`, or `mergeable_state` `draft`) | blocks (6), blocker `draft` |
| `pr`: `mergeable_state` `clean` or `has_hooks` | passes (the merge gate is then asked) |
| `pr`: `mergeable_state` `unstable` | pending: a check that is not required has not passed |
| `pr`: `mergeable_state` `blocked` while anything else is pending | pending |
| `pr`: `mergeable_state` `blocked` once every check read passed and every thread is resolved | the pull request is read again, since its first read came before the checks and a last check that finished in between leaves it stale; still open, unmerged and `blocked` at the same head, it blocks (6), blocker `merge_state`: branch protection the checks read do not show, such as a required review, or a required check that never reported; otherwise pending, and the next poll judges afresh |
| `pr`: `mergeable_state` `behind`, or any state not listed here | blocks (6), blocker `merge_state` naming it |
| `pr`: an unresolved review thread, outdated or not | blocks (3) |
| `pr`: merged | clear, nothing else read |
| `pr`: closed without merging | query error (7): nothing is left to wait for |
| `deploy`: latest status `success` | clear |
| `deploy`: `queued`, `pending`, `in_progress`, or no status yet | pending |
| `deploy`: `failure`, `error`, `inactive` or any other state | blocks (4); `inactive` means the deployment watched is no longer the live one |

An unknown value blocks rather than passes. At most 100 check runs, 100 commit statuses and 100
review threads are read; a target with more fails with exit 7 rather than being judged on part
of them. The proxy's thread route (below) gives no total, so a list of 100 or more from it
fails too, since it may have been cut short. A `ci` ref that is empty, `.` or `..` exits 7
before anything is read.

## Gates (exit 6)

Three blockers exit 6. A draft is blocker `draft`. A `mergeable_state` GitHub will not merge
(see the table above) is blocker `merge_state`, naming the state. Neither asks the merge gate,
since GitHub would refuse the merge anyway.

For `pr`, once GitHub shows nothing blocking and nothing pending, the watch asks the merge gate
of #33 (`docs/verdict.md`) whether `gh pr merge <n> --match-head-commit <head>` would be
allowed, with `<head>` the pull request's head commit on GitHub, judged from the working
directory's worktree and ledger exactly as the `merge-gate` hook judges a merge. A denial is
blocker `gate` with the gate's check id (`no-verdict`, `stale`, `changes-requested`, `tests`,
`review`, `unjudged`) and its one-line reason, which names the command that clears it. The
gate's own failures (no worktree, a ledger or setting it cannot read, a head commit that has
not been fetched) are denials too, so they exit 6: the merge would be denied for them. Fetch the
head, record the verdict, then watch again.

## Output

The envelope's `data`:

| Field | Value |
|---|---|
| `target` | `{"kind": "pr", "number": 7}`, `{"kind": "ci", "ref": "main", "repo": "octo/app" or null}` or `{"kind": "deploy", "id": "42", "provider": "github", "repo": ...}` |
| `state` | `clear`, `blocked` or `timeout` |
| `head` | the full commit id the checks were read for: the pull request's head, or the commit the `ci` ref resolved to (`null` for a deployment) |
| `blockers` | each blocker: `{"kind": "conflict", "pr", "url"}`, `{"kind": "thread", "url", "path"}`, `{"kind": "check", "name", "conclusion", "url"}`, `{"kind": "deployment", "id", "state", "url"}`, `{"kind": "draft", "pr", "url"}`, `{"kind": "merge_state", "pr", "state", "url"}` or `{"kind": "gate", "check", "reason"}` |
| `pending` | what was unfinished at the last poll: `{"kind": "check", "name", "status"}`, `{"kind": "no_checks"}`, `{"kind": "mergeable"}`, `{"kind": "merge_state", "state"}` or `{"kind": "deployment", "id", "state"}` |
| `polls`, `waited_secs`, `timeout_secs` | how many polls were made, how long the watch took, and the limit |

`findings` has one entry per blocker, keyed `watch/<kind>:<name>` (`watch/check:lint`,
`watch/thread:<url>` (`watch/thread:no-comment` for a thread with no comment),
`watch/conflict:7`, `watch/draft:7`, `watch/merge-state:blocked`, `watch/gate:no-verdict`,
`watch/deployment:42`), and on a timeout one per pending item (`watch/pending:...`).
Keys are distinct: a key already used in the envelope (two threads with no comment, a check run
and a status of one name) gets the first free suffix from `:2`, in the order listed.

## How GitHub is read

Only through `gh`, read-only, with the same bounded runner as `dag`'s GitHub tracker (30 s and
16 MiB per call):

- the pull request: `GET repos/{owner}/{repo}/pulls/<n>`;
- for `ci`, the commit its ref names, once per poll: `GET repos/<repo>/commits/<ref>?per_page=1`
  (one changed file asked for, to keep the answer small). The ref is percent-encoded into its
  path segment, so it cannot reach another endpoint or fill one of gh's
  `{owner}`/`{repo}`/`{branch}` placeholders. Both lists below are then read for that commit,
  so a branch that moves between the reads cannot mix two commits; `head` reports it;
- a commit's checks: `GET repos/<repo>/commits/<sha>/check-runs?per_page=100` (the latest run
  of each check) and `GET repos/<repo>/commits/<sha>/status?per_page=100` (the latest status of
  each context), for `pr` the pull request's head;
- review threads: one GraphQL query (`gh api graphql`, a query, never a mutation), since the
  REST API has no resolved state for a thread. Claude Code's cloud sessions refuse GraphQL and
  serve the same data on their GitHub proxy at
  `GET repos/{owner}/{repo}/pulls/<n>/ccr/review_threads`; when the query fails, that route is
  read instead, and when it fails too (as it does outside that proxy), the error (exit 7) names
  both: the GraphQL failure, then the route's (`...; the review thread route of Claude Code's
  proxy failed too: gh api .../ccr/review_threads: not found`);
- a deployment: `GET repos/<repo>/deployments/<id>/statuses?per_page=1`, newest first.

`{owner}` and `{repo}` are gh's placeholders for the current repository unless `--repo` names
one. A pull request poll makes four calls (five when GraphQL is refused and the proxy route is
read, and one more when a `blocked` pull request is read again); a commit poll three; a
deployment poll one.

## Ctrl-C

`watch` installs no signal handler: Ctrl-C (SIGINT) ends it at once, wherever it is, with the
signal's status (130 in a shell; `STATUS_CONTROL_C_EXIT` on Windows), never 0, and nothing on
stdout. It creates no temporary file, so nothing is left behind. A terminal's Ctrl-C reaches a
`gh` call in flight too, since it is in the same process group. The 30 s limit on a `gh` call
is kept by `anneal`, which kills the call when it passes, so a `gh` left running by a signal
sent to `anneal` alone (`kill -INT <pid>`) has no limit: it runs until `gh` itself finishes. A
watch started where SIGINT is ignored (a background job in a non-interactive shell) cannot be
interrupted this way; it still ends at `--timeout`, plus the poll in progress.

## Known gaps

- `GH_REPO` in the environment points `watch pr` at another repository while the gate still
  reads this worktree's ledger, as the merge gate's own known gap (`docs/verdict.md`).
- The thread fallback depends on the Claude Code proxy's route, which is not a GitHub API.
- Without branch protection that requires the checks, GitHub reports `clean` before CI has
  created its check runs, so a pull request whose one fast status or check has passed, before
  the rest of CI has registered, can exit 0.
- Pagination is not followed: more than 100 check runs, statuses or threads exit 7.
- Only GitHub Deployments is a deployment provider so far; another one implements
  `watch::deploy::DeployProvider` and gets its own `--provider` value.
