# Verdicts, the skip-rule and the merge gate

A pull request merges only with evidence for the code being merged: a fresh verdict that its
tests were verified, and a review verdict unless the change is small and touches nothing
sensitive. Three pieces carry this (#33):

- `anneal verdict set` records a verdict as a `verdict_set` event in the ledger;
- `anneal verdict valid` and `anneal ship skip-rule` answer, as exit codes, whether a pull
  request has a valid verdict and whether its change may skip full review;
- `anneal hook merge-gate`, a fail-closed `PreToolUse` hook on Bash, denies `gh pr merge` until
  both hold.

## `anneal verdict set`

```
anneal verdict set --pr <n> --sha <rev> --level <tests-verified|reviewed|approved>
                   --source <local|review-plugin|human> [--base <ref>] [--session <id>]
```

It resolves `--sha` to a full commit id, takes the base from `--base` (else `origin/main`, else
`main`), and records `verdict_set` with:

| Field | Value |
|---|---|
| `pr` | the pull request number (also the `pr:<n>` tag) |
| `verdict` | `tests_verified`, `reviewed` or `approved`: the levels, lowest first |
| `source` | `local` (anneal's own checks, such as `verify`), `review_plugin` or `human` |
| `sha` | the full commit id the verdict is for |
| `patch_id` | `git patch-id --stable` of `git diff <base>...<sha>`; `null` for an empty diff |
| `base` | the base as given or defaulted |

The review plugin records its outcomes, `changes_requested` and `noise_only`, with
`anneal ledger append`; `verdict set` records only levels. It writes to the ledger the hook reads
(no `--store`), and checks `--pr` and `--session` as ledger tags before it opens the ledger, so a
refused value creates no file. A commit or base that does not resolve is a usage error (2).

## Freshness

A verdict is fresh for a head when it was recorded for that commit, or when its patch id is the
head's patch id from the same base. The patch id hashes the diff's content without line numbers
or commit ids, so rebasing the branch onto a newer base, or recreating the same commits, keeps a
verdict fresh; changing a line of the change does not.

The diff is fixed so the id depends only on content: `git diff --no-color --no-ext-diff
--no-textconv --no-renames --binary -U3 --src-prefix=a/ --dst-prefix=b/ <base>...<head>`, piped
into `git patch-id --stable`. Each git call is bounded (10 s for the commands, 1 s in the hook;
the diff's output is not buffered, and `--numstat` is capped at 16 MiB). The patch id is computed
only when some verdict could be fresh by it.

The verdicts of a pull request are its latest 64 `verdict_set` events, in ledger order. Of the
fresh ones:

- a `changes_requested` voids the levels and reviews before it;
- the latest level after it is the tests verdict: `tests_verified` or better;
- the latest review after it is the review verdict: the review plugin's `reviewed`, `approved`
  or `noise_only`, or, while `review.require_plugin` is false, a human's `approved` (the interim
  rule, Ryan, 2026-10-11, until the review plugin exists).

## `anneal verdict valid`

```
anneal verdict valid --pr <n> [--head <rev>] [--base <ref>]
```

`--head` defaults to `HEAD`; the base defaults to the latest verdict's. It never creates a
ledger.

| Exit | Meaning |
|---|---|
| 0 | the latest fresh verdict is a level |
| 1 | no verdict (`invalid: no_verdict`), every verdict is stale (`stale`), or the latest fresh one requests changes (`changes_requested`); a `verdict/invalid:<n>` finding names the next step |
| 3 | the latest fresh verdict is `noise_only` |

`data` holds `pr`, `head`, `base`, `patch_id`, `require_plugin`, the latest fresh `verdict`
(`position`, `verdict`, `source`, `sha`, `fresh`: `by_sha` or `by_patch_id`), `invalid`, and
whether a tests verdict (`tests`) and a review verdict (`review`) hold, with the number of
`stale` verdicts.

## `anneal ship skip-rule`

```
anneal ship skip-rule [--base <ref>] [--head <rev>]
```

It reads `git diff --numstat --no-renames <base>...<head>` (base `--base`, else `origin/main`,
else `main`; head default `HEAD`) and exits 0 (skip full review) when the change touches at most
2 files, fewer than 50 lines (added plus removed), no binary file and no path matching a
sensitive glob; otherwise 1 (full review), with `data.reasons` listing each reason:
`too_many_files`, `too_many_lines`, `binary` (`path`) and `sensitive` (`path`, `glob`,
`category`). At most 64 binary and sensitive paths are listed; the rest are counted in `omitted`.
A rename counts as a deletion and an addition, so moving a sensitive file is sensitive.

The default sensitive globs, matched ignoring ASCII case, with `**` for any number of whole
directories and `*` and `?` within one name:

| Category | Globs |
|---|---|
| auth | `**/*auth*`, `**/*auth*/**`, `**/*login*`, `**/*login*/**`, `**/*password*`, `**/*credential*`, `**/*secret*`, `**/*secret*/**` |
| payments | `**/*payment*`, `**/*payment*/**`, `**/*billing*`, `**/*billing*/**`, `**/*invoice*`, `**/*checkout*`, `**/*stripe*` |
| migrations | `**/migrations/**`, `**/migrate/**`, `**/*migration*`, `**/*.sql` |
| ci | `.github/**`, `.gitlab-ci.yml`, `.gitlab/**`, `.circleci/**`, `.buildkite/**`, `.travis.yml`, `azure-pipelines.yml`, `**/Jenkinsfile` |
| infra | `**/*.tf`, `**/*.tfvars`, `**/terraform/**`, `**/infra/**`, `**/infrastructure/**`, `**/k8s/**`, `**/kubernetes/**`, `**/helm/**`, `**/ansible/**`, `**/deploy/**`, `**/Dockerfile*`, `**/docker-compose*` |
| gate config | `.anneal/**`, `.claude/**` |

`**/*auth*` also matches `author.rs`: a false full review is the cheap mistake.

## `.anneal/config.json`

```json
{
  "review": {"require_plugin": false},
  "ship": {"sensitive_globs": ["src/crypto/**", "**/*.pem"]}
}
```

`review.require_plugin` (default `false`) decides whether a human `approved` counts as the review
verdict. `ship.sensitive_globs` (at most 256, each at most 256 bytes, relative, `/`-separated)
adds globs to the defaults under the category `configured`; it cannot remove one. Unknown keys
inside `review` and `ship` are refused; other top-level keys (`check_fast`) belong to other
readers. The file must be a regular file of at most 64 KiB. A file that does not parse is a
usage error for the commands and a denial in the hook.

## The merge gate

`anneal hook merge-gate --fail-closed` runs on every Bash call (`PreToolUse`, matcher `Bash`,
5 s, `onFailure: "block"`). It reads the command as the guard does, through wrappers, `bash -c`,
`eval`, scripts on stdin and variables the command sets, and finds each `gh pr merge` it runs. A
call that runs none is allowed silently; so is `gh pr merge --disable-auto`, which merges nothing.
For each merge:

1. The pull request must be named by number (`7`, `#7` or a `.../pull/7` URL). With no argument
   gh merges the current branch's pull request, and a branch name needs a network lookup; both
   are denied, as is `-R`/`--repo` (another repository's verdicts are not in this ledger).
2. The head is `--match-head-commit` when given (gh refuses the merge if the remote head
   differs), else the worktree's `HEAD`.
3. A fresh tests verdict must hold for the head. Otherwise it denies with `no-verdict`, `stale`,
   `changes-requested` or `tests`, naming `anneal verdict set ... --level tests-verified`.
4. A review verdict must hold, unless the skip-rule skips the change from the base the latest
   verdict recorded (else the default base). Otherwise it denies with `review`, naming the review
   plugin and, while `review.require_plugin` is false, `anneal verdict set ... --level approved
   --source human`.

A denial is Claude Code's deny decision whose reason is one `ERROR merge-gate: ...` line, plus a
`gate_blocked` event (`point` `merge-gate.<check>`) in the ledger when one exists; the gate never
creates a ledger, and with none every pull request has no verdict. Unreadable input or commands,
a merge naming no number, and a ledger, git or `.anneal/config.json` it cannot read are denied.
git and ledger reads share 4 s from the hook's start, each git call at most 1 s, and each ledger
statement waits at most 500 ms for a lock.

### Known gaps

The gate judges what it can see in a Bash command, for an honest agent:

- merges outside `gh pr merge`: `gh api` calls to the merge endpoint, a gh alias, the web UI, or
  another tool;
- `GH_REPO` naming another repository (only `-R`/`--repo` is read);
- the head is the local `HEAD` unless `--match-head-commit` pins it, so a branch pushed from
  elsewhere is judged by the local checkout;
- `.anneal/config.json` is read from the working tree, so an uncommitted edit changes the
  settings; and anyone can record `--source human` or append a verdict with `anneal ledger
  append`.
