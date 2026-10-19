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
                   --source <local|review-plugin|human> [--actor human|model]
                   [--via mod|bash|terminal] [--base <ref>] [--session <id>]
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
| `by` | who ran it (`actor`: `human` or `model`) and through what (`via`: `mod`, `bash` or `terminal`), each only when given; absent with neither (#62) |

The review plugin records its outcomes, `changes_requested` and `noise_only`, with
`anneal ledger append`; `verdict set` records only levels. It writes to the ledger the hook reads
(no `--store`), and checks `--pr` and `--session` as ledger tags before it opens the ledger, so a
refused value creates no file. A commit or base that does not resolve is a usage error (2).

`--actor human` is a person's to give: the guard denies a model's Bash call that runs
`verdict set ... --actor human` (rule `human-only`, `docs/guard.md`), so a human verdict comes
from the cockpit mod (`--actor human --via mod`) or the person's own terminal; `anneal ledger
append` refuses a `verdict_set` with a `by` (`docs/ledger.md`). The merge gate
does not read `by` yet: a human `approved` counts as the review by its `source`, as before.
Counting it only with `actor: human` would change existing tests' expectations and is open for
Ryan (#62).

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
5 s, `onFailure: "block"`). It judges a merge only in one shape, the canonical merge command,
run as a Bash call of its own:

```
gh pr merge <number> --match-head-commit <full commit> --repo <host>/<owner>/<repo>
```

with its arguments in any order, and optionally `--squash`, `--merge`, `--rebase`,
`--delete-branch`, `--auto`, `--subject <text>` and `--body <text>`. `--repo` may also be
written `--repo=<value>`, `-R <value>`, `-R<value>` or `-R=<value>`, and `--match-head-commit`
as `--match-head-commit=<value>`; when an option is given twice the last one counts, as in gh's
flag parser (pflag). The shape is read from the command's raw text: every word is plain
(letters, digits and `._/:@%+,=-`), so bash neither expands, splits nor drops it; only a
`--subject` or `--body` text may be quoted, in single quotes or in double quotes holding no `$`,
backtick, `\` or `!`, so it stays one word and runs nothing. There is nothing before or after
the merge: no separator, pipe, redirection, assignment, wrapper, subshell, heredoc, comment,
variable, glob, brace, `~` or second command.

A merge is judged only as the canonical command; any other merge is denied. Any other command
is read as the guard reads commands (wrappers, `bash -c`, `eval`, scripts on stdin, variables
the command sets, gh's options as pflag reads them). If that read finds a `gh pr merge` in it,
or cannot read it (`unreadable`), it is denied: with `env` when it sets `GH_REPO` or `GH_HOST`
(brace expansion included; a brace sequence that may build such a name, `GH_REP{O..O}`, is
unreadable), `directory` when it changes directory anywhere (`cd`, `pushd`, `popd`, a function or
trap body defined before a later `cd`, `env -C`/`--chdir`, `sudo -D`/`--chdir`, a login shell,
`find -execdir`/`-okdir`), `target` when the merge names no pull request number, and otherwise
`shape`, naming the canonical command.

The read can miss a merge whose words exist only at run time, so one tripwire backs it, also
denying with `shape`:

- a gh call whose first or second operand is known only at run time (`gh "$@"`,
  `gh $(cat args)`), whatever else the command holds;
- a command one of whose words holds `merge` in any case, after quote removal (`'mer'ge` and
  `me\rge` read `merge`; words inside `$( )`, backticks and `bash -c` strings count, and so do
  here-documents and here-strings), together with any of: a word naming gh (its last path
  component is `gh` or `gh.exe`, after the command's variables are put in) that the read took
  no full gh call from in the same simple command (`xargs gh`, `parallel gh`,
  `ln -s /usr/bin/gh g`, `command -v gh`; with `G=gh`, `$G pr view 1` is a full call and
  covers no other word); a `gh api` call (`gh api -X PUT repos/o/r/pulls/7/merge`); a program
  known only at run time (`${GH:-gh} pr merge 7`); or a definition (`f() { … }`, `function`,
  `alias`, `hash -p`, bare or under `builtin` or `command`, `BASH_FUNC_`).

`git merge gh-pages` passes, since no word is exactly `gh`. Every other command is allowed
silently, as is `gh pr merge --disable-auto` (bare, or set to a true value such as `1` or
`true`), which merges nothing.

**Threat model.** The canonical command is the only path that gets judged. The fallback is a
best-effort tripwire against mistakes and casual workarounds; it is not complete against a
determined adversary in a Turing-complete shell, and Known gaps lists what it cannot see.
Strong enforcement is a GitHub required check or branch protection on the repository, which is
a follow-up. For the canonical command:

1. The pull request must be named by number (`7`, or an `http(s)://<host>/<owner>/<repo>/pull/7`
   URL whose host, owner and repository are the worktree's `origin`; an unquoted `#7` starts a
   shell comment). With no argument gh merges the current branch's pull request, and a branch
   name needs a network lookup; both are denied (`target`), as is any other URL (`repo`).
2. The head must be pinned with `--match-head-commit <full 40-hex commit>`, so gh refuses the
   merge if the remote head differs from the commit judged. Without a pin the denial names the
   command to run, with the worktree's `HEAD` (and `--repo`) filled in.
3. `--repo` must name the repository the gate judges, the worktree's `origin`, as
   `<host>/<owner>/<repo>`, compared without case. This is what the gate's invariant rests on:
   gh takes the repository from `--repo` before `GH_REPO` and before the working directory's
   git remotes, and takes the host in it before `GH_HOST` (a bare `owner/repo` would take
   `GH_HOST`'s, so it is denied). With the canonical command run on its own, nothing in the
   command can change which repository, pull request or head gh receives. A merge without
   `--repo`, with another repository, or in a worktree whose `origin` does not read as
   `<host>/<owner>/<repo>` is denied (`repo`), and the denial names the exact command:
   `gh pr merge <n> --match-head-commit <HEAD> --repo <origin>`. `origin` and `--repo` are read
   by one rule: each part in `[A-Za-z0-9._-]`, and a `.git` ending, in any case, dropped from
   `origin` and refused in `--repo`.
4. A fresh tests verdict must hold for the head. Otherwise it denies with `no-verdict`, `stale`,
   `changes-requested` or `tests`, naming `anneal verdict set ... --level tests-verified`.
5. A review verdict must hold, unless the skip-rule skips the change from the base the latest
   verdict recorded (else the default base). Otherwise it denies with `review`, naming the review
   plugin and, while `review.require_plugin` is false, `anneal verdict set ... --level approved
   --source human`.

A denial is Claude Code's deny decision whose reason is one `ERROR merge-gate: ...` line, plus a
`gate_blocked` event (`point` `merge-gate.<check>`, the checks being `shape`, `target`, `repo`, `env`,
`directory`, `head`, `no-verdict`, `stale`, `changes-requested`, `tests`, `review`, `bad-input`, `unreadable`
and `unjudged`) in the ledger when one exists; the gate never creates a ledger, and with none
every pull request has no verdict. Unreadable input or commands, a merge naming no number, and a
ledger, git or `.anneal/config.json` it cannot read are denied. git and ledger reads share 3.5 s
from the hook's start, each git call at most 1 s, and each ledger statement waits at most 500 ms
for a lock and never past what is left of the 3.5 s.

### Known gaps

The gate judges what it can see in a Bash command, for an honest agent:

- what the command text cannot show: a function or alias for `gh` (or `command`, `builtin`)
  defined in a shell startup file or in Claude Code's shell snapshot (for example
  `alias gh='op plugin run -- gh'`), and `PATH`, `GH_REPO`, `GH_HOST` or `BASH_ENV` set in the
  session's environment rather than in the command. The canonical command runs whatever `gh`
  means in that shell;
- merges outside `gh pr merge`: a gh alias (`gh alias set`), `gh api` calls to the merge
  endpoint (caught when the text holds `merge`; not when the path is built at run time without
  it), the web UI, or another tool;
- interpreters and words built at run time: a merge run by `python -c`, `perl -e`, `node -e`,
  make, or a script on disk, or built at run time from words that do not hold the literal text
  `merge` or name `gh` (`m=$(printf 'm%serge' ''); $EDITOR pr $m 7` with `EDITOR=gh`, read
  from a file, decoded), is outside what the command text shows. The gate reads the text, not
  what runs;
- the tripwire's false positives: a command that holds `merge` is denied too when it names gh
  outside a call the read takes in full (`echo gh pr merge 3`,
  `gh pr view 7 | xargs gh pr comment -b merged`), runs `gh api` (`gh api repos/o/r/merges`),
  runs a program known only at run time
  (`$(git rev-parse --show-toplevel)/scripts/check.sh && git merge --ff-only main`,
  `"$CARGO" test && git merge main`) or holds a definition (`f() { git merge "$1"; }; f main`,
  `alias gs='git status'; git merge main`, and `[[ ( -f x ) ]] && git merge main`, whose
  `( -f x )` reads as a function definition). Split such a command into separate Bash calls,
  keeping the part that holds `merge` apart from the gh word, run-time program or definition;
- an `origin` that names an ssh host alias (`git@github-work:org/repo.git`, with
  `github-work` defined in `~/.ssh/config`) reads as `github-work/org/repo`, a host gh cannot
  reach, so every merge in that worktree is denied or fails. The gate stays offline and does
  not resolve aliases (no `ssh -G`). Point `origin` at the real host
  (`git remote set-url origin git@github.com:org/repo.git`), or map the alias to it with
  `git config url.git@github.com:.insteadOf git@github-work:`, which `git remote get-url`
  expands, so the gate reads `github.com/org/repo` (git then connects to the real host too, so
  choose the alias's key another way, such as `core.sshCommand`);
- the gate judges the worktree it runs in: `origin` and the ledger are read from the hook's
  working directory, so a Bash call that an earlier call moved into another repository's
  worktree is judged against that repository, and `--repo` must then name it;
- freshness by patch id trusts the local base ref: the base is recorded as a name
  (`origin/main`) and resolved again when the merge is judged, so moving that ref locally
  changes the diff the head's patch id is taken from. Recording the base's commit and requiring
  it to be an ancestor of the judged base would close this, but needs a new `verdict_set` field
  (a registry change); the commit pin still makes gh refuse a head other than the one judged;
- `.anneal/config.json` is read from the working tree, so an uncommitted edit changes the
  settings; and anyone can record `--source human` or append a verdict with `anneal ledger
  append`.
