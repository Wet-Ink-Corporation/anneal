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
| `patch_id` | `git patch-id --verbatim` of `git diff <base>...<sha>`; `null` for an empty diff |
| `base` | the base as given or defaulted |

The review plugin records its outcomes, `changes_requested` and `noise_only`, with
`anneal ledger append`; `verdict set` records only levels. It writes to the ledger the hook reads
(no `--store`), and checks `--pr` and `--session` as ledger tags before it opens the ledger, so a
refused value creates no file. A commit or base that does not resolve is a usage error (2).

## Freshness

A verdict is fresh for a head when it was recorded for that commit, or when its patch id is the
head's patch id from the same base. The patch id hashes the diff's content without line numbers
or commit ids, so rebasing the branch onto a newer base, or recreating the same commits, keeps a
verdict fresh; changing a line of the change does not, and neither does changing only its
whitespace (`--verbatim` hashes whitespace as written, since it is code in Python, YAML and
Makefiles).

The diff is fixed so the id depends only on content: `git diff --no-color --no-ext-diff
--no-textconv --no-renames --ignore-submodules=none --binary -U3 --src-prefix=a/
--dst-prefix=b/ <base>...<head>`, piped into `git patch-id --verbatim`. A submodule pointer
change counts whatever `diff.ignoreSubmodules` the repository sets, in the patch id and in the
skip-rule. Each git call is bounded (10 s for the commands, 1 s in the hook;
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
| auth | `**/*auth*`, `**/*auth*/**`, `**/*login*`, `**/*login*/**`, `**/*password*`, `**/*password*/**`, `**/*credential*`, `**/*credential*/**`, `**/*secret*`, `**/*secret*/**` |
| payments | `**/*payment*`, `**/*payment*/**`, `**/*billing*`, `**/*billing*/**`, `**/*invoice*`, `**/*invoice*/**`, `**/*checkout*`, `**/*checkout*/**`, `**/*stripe*`, `**/*stripe*/**` |
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
`eval`, scripts on stdin and variables the command sets, and finds each `gh pr merge` it runs.
gh's arguments are read as gh's flag parser (pflag) reads them: options may come before `pr`,
short options cluster (`-db 5` is `-d -b 5`), and `-R=o/r` is `-R o/r`. A call that runs none is
allowed silently; so is `gh pr merge --disable-auto` (bare, or set to a true value such as `1`
or `true`), which merges nothing; `--disable-auto=false` is a merge. For each merge:

1. The pull request must be named by number (`7`, `#7`, or an `http(s)://<host>/<owner>/<repo>/pull/7`
   URL whose host, owner and repository are the worktree's `origin`). With no argument gh merges
   the current branch's pull request, and a branch name needs a network lookup; both are denied,
   as are `-R`/`--repo`, any other URL, and a `GH_REPO` or `GH_HOST` the command sets, also
   through brace expansion (`export GH_RE{PO,X}=o/x`): another repository's verdicts are not in
   this ledger. gh finds its repository from the working directory's git remotes, so a merge the
   command runs after changing directory is denied too (`directory`): after a `cd`, `pushd` or
   `popd` anywhere before it, under `env -C`/`--chdir` or `sudo -D`/`--chdir`, or as a
   `find -execdir`/`-okdir` action.
2. The head must be pinned with `--match-head-commit <full 40-hex commit>`, so gh refuses the
   merge if the remote head differs from the commit judged. Without a pin the denial names the
   command to run, with the worktree's `HEAD` filled in.
3. A fresh tests verdict must hold for the head. Otherwise it denies with `no-verdict`, `stale`,
   `changes-requested` or `tests`, naming `anneal verdict set ... --level tests-verified`.
4. A review verdict must hold, unless the skip-rule skips the change from the base the latest
   verdict recorded (else the default base). Otherwise it denies with `review`, naming the review
   plugin and, while `review.require_plugin` is false, `anneal verdict set ... --level approved
   --source human`.

A denial is Claude Code's deny decision whose reason is one `ERROR merge-gate: ...` line, plus a
`gate_blocked` event (`point` `merge-gate.<check>`, the checks being `target`, `repo`, `env`,
`directory`, `head`, `no-verdict`, `stale`, `changes-requested`, `tests`, `review`, `bad-input`, `unreadable`
and `unjudged`) in the ledger when one exists; the gate never creates a ledger, and with none
every pull request has no verdict. Unreadable input or commands, a merge naming no number, and a
ledger, git or `.anneal/config.json` it cannot read are denied. git and ledger reads share 3.5 s
from the hook's start, each git call at most 1 s, and each ledger statement waits at most 500 ms
for a lock and never past what is left of the 3.5 s.

### Known gaps

The gate judges what it can see in a Bash command, for an honest agent:

- merges outside `gh pr merge`: `gh api` calls to the merge endpoint, a gh alias, the web UI, or
  another tool;
- `GH_REPO` or `GH_HOST` already in the process environment (only an assignment in the command
  is seen), and a gh configured for another host than `origin`'s;
- a working directory changed by a program the guard does not know (`chroot`, `nsenter --wd`,
  `systemd-run --working-directory`) or by an earlier Bash call, and `GIT_DIR` or
  `GIT_WORK_TREE` pointing gh's git at another repository;
- freshness by patch id trusts the local base ref: the base is recorded as a name
  (`origin/main`) and resolved again when the merge is judged, so moving that ref locally
  changes the diff the head's patch id is taken from. Recording the base's commit and requiring
  it to be an ancestor of the judged base would close this, but needs a new `verdict_set` field
  (a registry change); the commit pin still makes gh refuse a head other than the one judged;
- `.anneal/config.json` is read from the working tree, so an uncommitted edit changes the
  settings; and anyone can record `--source human` or append a verdict with `anneal ledger
  append`.
