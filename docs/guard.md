# The guard hook

`anneal hook guard` runs before every Bash tool call and refuses a short list of commands that
damage a repository or weaken its gates. It is the cheapest step of the escalation ladder:
script, then Clef, then a model judge, then a human. It is deterministic. It reads the command
the way a shell does and does not use a regular expression on the raw text, so quoting, chains
and wrappers do not get a command past it.

## How it runs

`plugin/hooks/hooks.json` declares it under `PreToolUse` with matcher `Bash`:

```json
{ "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/libexec/anneal",
  "args": ["hook", "guard", "--fail-closed"], "timeout": 5, "onFailure": "block" }
```

- **Allowed:** no output and exit 0.
- **Asked** (only Clef D1 in enforce mode, see [Clef D1](#clef-d1-destructive_or_irreversible-30)):
  one `PreToolUse` decision with `"permissionDecision": "ask"` and a one-line reason, exit 0.
- **Denied:** one JSON object on stdout and exit 0. Claude Code shows the reason to the model,
  and the reason is one `ERROR <reason>` line naming the rule:

  ```json
  {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny",
   "permissionDecisionReason":"ERROR guard force-push: force push to protected branch main (main, master and release/* are protected)"}}
  ```

  The guard also appends a `gate_blocked` event to the ledger, with `point` set to
  `guard.<rule>`, the reason, actor `hook` and the session id. If the ledger cannot take the
  event, the command is still denied, and the reason ends with
  `(the block was not recorded in the ledger: ...)`. Recording is bounded so the deny always
  arrives inside the hook's 5 s `timeout`: git gets 1 s to find the ledger and each ledger
  statement waits at most 500 ms for another process's lock (`a_git_that_never_answers_still_gets_the_deny_out_within_the_hook_budget`).

### Failing closed

Whatever the guard cannot judge, it denies:

| Failure | Result |
|---|---|
| stdin is not `PreToolUse` JSON, is larger than 1 MiB, or cannot be read | deny, rule `bad-input` |
| a Bash call with no `tool_input.command` | deny, rule `bad-input` |
| the command cannot be read: an unclosed quote, `$(`, backtick or `${`, a `case` with no `esac` inside `$( )` (any unquoted `case` word counts, so `$(echo case)` is denied too), a redirection with no target, nesting deeper than 32 | deny, rule `unparseable` |
| `bash -c`, `eval`, `env -S` or `find -exec` nested deeper than 8, or more than 4 MiB lexed in total across those levels | deny, rule `unparseable` |
| a form the guard does not model (see [Bypass handling](#bypass-handling)): a script from stdin or a process substitution known only at run time, a program word built by brace expansion or from a command's output, a `git -c alias.*` | deny, rule `unparseable` |
| brace expansion that makes more than 256 words | deny, rule `unparseable` |
| the arguments of programs the guard does not know read as a command more than 32 times in one check | deny, rule `unparseable` |
| the decision cannot be written to stdout | exit 2 with one `ERROR` line on stderr (`docs/exit-codes.md`) |
| bad arguments (a usage error) | exit 2 |
| no anneal binary is installed | the launcher exits 2 with one `ERROR` line (`--fail-closed`) |
| a crash, a timeout (5 s), any other exit code, or invalid JSON | Claude Code blocks the call (`onFailure: "block"`, Claude Code 2.1.295 or later) |

Claude Code reads exit 2 as "block". Tests pin these cases: `cargo test -p anneal-cli guard`,
`tests/it/hook_guard.rs`, `tests/it/hooks_manifest.rs` and `scripts/test-launcher.sh`.

Tool calls other than Bash are not judged. The matcher never sends them, and if one arrives it
is allowed.

## The denylist

Each entry lists what it denies and the near misses it lets through. The table tests in
`crates/anneal-cli/src/hook/guard/denylist/tests.rs` and `tests/` beside it hold every row
below and more; `tests/forms.rs` checks every entry in every wrapping form of the bypass table.

### `force-push`: force push to a protected branch

A `git push` is forced when it has `--force`, `-f` (also inside a short-option cluster such as
`-fu`), `--force-with-lease[=...]`, `--mirror`, or a refspec that starts with `+`. Long options
match when abbreviated, as git and `getopt_long` accept any unique prefix (`--forc`,
`--force-w`, `--mirr`); a prefix several options share (`--f`) counts as the one denied. A forced
push is denied when:

- a forced refspec's destination is protected: `main`, `master`, `release/*`, with or without
  `refs/heads/`, or a pattern such as `refs/heads/*`;
- a forced refspec has no explicit destination: `HEAD`, `@`, a revision expression (`~`, `^`,
  `@{`), or a variable;
- it has no refspec at all, because it then pushes the current branch, which may be protected;
- it pushes every branch (`--all`, `--branches`, `--mirror`).

Allowed: any push that is not forced, and a forced push whose every forced refspec names
another branch (`git push --force origin feature`, `git push -f origin HEAD:feature`,
`git push origin +feature main`). Git's global options (`-C`, `-c`, `--git-dir`, ...) are
skipped before the subcommand, after these checks on `-c` and `--config-env`:

- a setting that defines an alias (`-c alias.p='push -f'`, any case), or one whose key is only
  known at run time (`-c "$X"`), is denied as `unparseable`: the alias is a git command the guard
  cannot see;
- a setting whose value git runs as a command (`core.sshCommand`, `core.pager`, `core.editor`,
  `core.fsmonitor`, `core.askPass`, `sequence.editor`, `diff.external`, `credential.helper`,
  `gpg.program`, `pager.*`, `filter.*.clean|smudge|process`, `diff.*.command|textconv`,
  `merge.*.driver`, `credential.*.helper`, `gpg.*.program`) has its value checked as shell
  text (a leading `!` dropped), so `-c core.pager=cat` is allowed and
  `-c core.sshCommand='rm -rf /'` is not. With `--config-env` the value is in the
  environment, so such a key is denied as `unparseable`.

### `rm-rf`: recursive force delete outside temp and scratch

`rm` with both a recursive flag and a force flag, in any spelling (`-rf`, `-fr`, `-Rf`,
`-r -f`, `--recursive --force` or any abbreviation such as `--r --f`, flags after the
operands). It is allowed only when every
operand is a literal absolute path strictly inside a temporary or scratch directory:

- `$TMPDIR`, `/tmp`, `/var/folders`, `/private/tmp` or `/private/var/folders`;
- `$CLAUDE_PROJECT_DIR/.scratch`.

`$TMPDIR` and `$CLAUDE_PROJECT_DIR` are read from the hook's environment. Any other operand is
denied: a relative path, `/`, `.`, `..`, `~`, a glob or brace (`*`, `/tmp/*`, `/tmp/{a,b}`), a
variable or substitution (`"$TMPDIR/x"`), the directory itself (`/tmp`), or a path that leaves
it (`/tmp/../etc`). `..` is resolved without looking at the file system. Operands that `xargs`
or `find -exec` adds cannot be seen, so `rm -rf` under them is denied; what a static `echo` or
`printf` pipes into `xargs` is appended and checked (`echo '-rf /' | xargs rm`).

An operand that is wholly a variable or substitution (`$F`, `"$FLAGS"`) may hold the flags.
When another operand follows it, it is taken as `-rf`, so `F=-rf; rm $F /` and
`rm "$FLAGS" build` are denied while `rm -f "$tmp"` is allowed. A cost: `rm -f "$a" "$b"` is
denied too, since `$a` may be `-r`.

Allowed: `rm -rf /tmp/build`, `rm -r src` (not forced), `rm -f file` (not recursive), and
`rm`, `-rf` or `/` in a quoted string (`echo 'rm -rf /'`).

### `git-add-force`: `git add -f`

`git add` (or its synonym `git stage`) with `-f`, `--force` (or an abbreviation such as
`--forc`), or `f` in a short-option cluster (`-fA`). It stages files that `.gitignore` excludes. Allowed: `git add -- -f` (a path named
`-f`) and every other `git add`.

### `config-write`: Bash writes to lint or hook config

A Bash command may not change these paths. They are matched on path components, so
`docs/hooks.md` and `myhooks/x` do not match, and without regard to letter case, since on a
case-insensitive file system (APFS, NTFS) `.CLAUDE/Settings.json` is `.claude/settings.json`:

| Path | Matches |
|---|---|
| `hooks/` | any component named `hooks` (`hooks/x`, `.git/hooks/pre-commit`, `plugin/hooks/hooks.json`) |
| `.claude/settings*.json` | `settings.json`, `settings.local.json`, ... directly in `.claude` |
| `.claude/hooks/` | covered by `hooks/` |
| `clippy.toml`, `.clippy.toml`, `rustfmt.toml`, `.rustfmt.toml`, `deny.toml` | anywhere |
| `.cargo/config*` | `config`, `config.toml` directly in `.cargo` |
| `.github/workflows/` | `workflows` directly in `.github` |
| `.eslintrc*`, `.pre-commit-config.yaml`, `lefthook.yml` | anywhere |

The writes it checks:

- a redirection: `>`, `>>`, `>|`, `&>`, `&>>`, `<>`, `>&file`, with or without a descriptor
  number;
- `tee` and `truncate` operands;
- `sed -i` (also `-i.bak`, `--in-place`, `--in`) operands. The script is not a path, so
  `sed -i 's/hooks/x/' notes` is allowed;
- the destination of `cp`, `mv`, `ln` and `install`, including `-t DIR` and
  `--target-directory` (or `--target`, `--t`). When the destination
  is a directory (`-t`, a trailing `/`, or `.claude`, `.github`, `.cargo`), the destination is
  also checked joined with each source's name, so `cp settings.json .claude/` is denied;
- what `mv` moves away, and what `rm`, `unlink`, `rmdir`, `shred`, `chmod`, `chown` and `chgrp`
  touch. These also deny `.claude`, `.github` or `.cargo` itself, because removing or
  re-permissioning that directory changes the config inside it.

A path component with an unquoted glob is denied when it could match a protected name
(`.cl*/settings.json`). Reading these files (`cat`, `cp clippy.toml /tmp/x`) is allowed.

`..` is resolved lexically (`.claude/rules/../settings.json` is `.claude/settings.json`). A
relative path that climbs above where it starts may land under any parent, so it matches an
entry that needs a parent on its name alone: `../settings.json` counts as
`.claude/settings.json` and `../../workflows/ci.yml` as `.github/workflows/ci.yml`, and
`rm -r ..` is denied because the directory above may hold `.claude/`.

A `cd` (or `pushd`) earlier in the command moves where relative paths start. Each relative
target is checked as written and also joined to the directory the `cd` named, so
`cd .claude && echo x > settings.json` is denied (`cd` alone and `cd ~` go home). After a `cd`
to a variable or to `-`, relative paths may start anywhere, so they are matched as climbing
above their start (`cd "$DIR" && echo x > settings.json` is denied).

## Bypass handling

The guard is a denylist: defence in depth, not a sandbox. It models the shell features that
commonly hide a command, and where a form is beyond what it models, it fails closed and denies
the command (rule `unparseable`) rather than guessing. The command goes through a POSIX-ish
lexer (`hook/guard/shell.rs`), and then every simple command it runs is checked in order
(`hook/guard/denylist/checker.rs`) after it is unwrapped (`hook/guard/denylist/wrappers.rs`):

| Form | Handling |
|---|---|
| Quoting: `"git" push`, `g'i't`, `r\m`, `rm -r''f`, `$'\x72\x6d'` | quotes and escapes are removed before matching; ANSI-C escapes are decoded |
| Chains: `&&`, `\|\|`, `;`, `\|`, `\|&`, `&`, newlines, `\`-newline | each simple command is checked |
| Subshells, groups and functions: `( )`, `{ ...; }`, `if`/`then`/`while`/`do`/`!`, `f() { ...; }`, `function f { ...; }`, `coproc [NAME]` | flattened; leading keywords, `function NAME` and `coproc NAME` are skipped, so a function body is checked where it is defined |
| `case` | a pattern's `)` does not close an enclosing `$( )` or `${x:-$( )}`; every unquoted `case` word is counted (bash also opens one after `time -p`, `function f` or `coproc`), and `esac` in command position |
| Substitutions: `$( )`, backticks, `<( )`, `>( )`, `$(( ))`, `${x:-$(...)}` | the inner commands are checked; the word holding one is dynamic; a process substitution is a word (a pipe's path) |
| Heredocs | the body is data and the reading command's stdin; with an unquoted delimiter its substitutions are checked |
| `bash -c`, `sh -c`, `zsh -c`, `dash`, `ksh`, `mksh`, `ash` (also `-lc`, `-o opt -c`) | the string is lexed and checked, up to 8 levels deep and 4 MiB in total |
| A shell reading its script from stdin: `bash <<'EOF'`, `sh -s`, `bash <<< '...'`, `echo '...' \| sh`, `bash /dev/stdin`, `sudo -s`, `su`, `script` | the script is checked when it is static: a heredoc whose delimiter is quoted (or whose body has no substitution), a literal here-string, or what `echo` or `printf` with literal arguments (no `\` escape, no `%` directive) pipes in. Any other stdin (`curl ... \| sh`, `cat x \| bash`, `bash <<< "$S"`) is denied |
| A script from a process substitution or descriptor: `bash <(curl ...)`, `source <(...)`, `bash /dev/fd/3`, `bash < <(...)` | denied |
| `eval`, `alias NAME=VALUE`, `trap '...' SIG`, `su -c`, `script -c`, `watch`, `flock -c`, `env -S '...'` | the string is lexed and checked (`eval` and `watch` join their arguments) |
| Environment: `VAR=x cmd`, `env [-i] [-u N] VAR=x cmd` | assignments and options are skipped |
| Wrappers: `sudo`, `doas`, `command`, `builtin`, `exec`, `nohup`, `time`, `nice`, `timeout`, `setsid`, `stdbuf`, `ionice`, `chrt`, `taskset`, `unbuffer`, `busybox`, `flock`, `xargs`, `find -exec` | their options (and the duration, priority, mask or lock file) are skipped and the wrapped command is checked; every `-exec`, `-execdir`, `-ok` and `-okdir` of a `find` is checked |
| Unlisted wrappers: `caffeinate`, `runuser -u root --`, `chroot /`, `unshare`, `nsenter`, `systemd-run`, `pkexec`, `parallel ... :::` | the arguments of any program the guard does not know are read as a command from each word that names a checked program (`rm`, `git`, `tee`, `truncate`, `cp`, `mv`, `install`, `ln`, `sed`, `unlink`, `rmdir`, `shred`, `chmod`, `chown`, `chgrp`), a shell or a wrapper above. Programs that only print, search or test their arguments (`echo`, `printf`, `grep`, `rg`, `man`, `which`, `type`, `test`, `[`, `for`, `case`, ...) are skipped. The cost is some false denials: `cargo run -- rm -rf build` or `ssh host git push -f origin main` are denied |
| Long options | an abbreviation counts as the option (`--forc` is `--force`, `--comm` is `--command`), for the entries and for the wrappers that use `getopt_long`; the shells' own long options must be spelled out, as bash requires |
| Program paths: `/bin/rm`, `/usr/bin/git`, `git.exe` | only the last path component, without `.exe`, is used |
| Brace expansion: `git push {-f,origin,main}`, `tee .claude/{settings.json,x}` | arguments are expanded (up to 256 words) and the words keep their pattern shape; a program word built by brace expansion (`{rm,-rf,/}`) is denied |
| Variables set earlier in the command: `F='rm -rf /'; $F`, `T=.claude/settings.json; echo x > $T` | a standalone assignment (also after `export`, `declare`, `local`, `typeset`, `readonly`) with a literal value is put in where the variable is used, split into words when unquoted. A variable the command sets to a value known only at run time (`F=$(...)`, `read F`, `for F in`, `printf -v F`, `F+=x`, two different values) and then runs as the program is denied, as is a name reference or array (`declare -n`, `local -n`, `-a`, `-A`, alone or in a cluster such as `-gn`); after the command sets `IFS`, unquoted values are not put in |
| A program word that is only expansions: `$(echo rm) -rf /`, `"$GIT" push -f`, `rm${IFS}-rf${IFS}/` | its literal parts are read as words (each expansion a separator), and its arguments are checked against every entry. If it is a command's output (`$(...)`, backticks), the arguments of a function or script (`"$@"`, `$*`, `$1`) or an indirect expansion (`${!y}`), it is denied |

### Limits

The guard reads the command; it does not run it. It is one layer of several, so it denies what
it cannot model instead of modelling every shell feature, and it does not try to see:

- values from the environment (`"$EDITOR" file`, `$GIT status` run, with their arguments
  checked; `cp x "$DEST"` is allowed and `git push -f origin "$B"` denied);
- scripts in files and the code of other interpreters (`bash script.sh`, `source x`, `make`,
  `npm run`, `python -c`, `node -e`, `perl -e`);
- other programs that delete or write files (`find -delete`, `git clean`, `dd of=`,
  `curl -o`), and environment variables that make git run a command (`GIT_SSH_COMMAND`,
  `GIT_EDITOR`);
- `PATH` changes, functions or aliases defined by an earlier Bash call, and how a pipe's
  writer that is not a static `echo` or `printf` reaches `xargs`;
- symbolic links (`/tmp/link` that points elsewhere);
- a command string given to a program it does not know as one word (`ssh host 'rm -rf /'`,
  `tmux new 'git push -f'`): only words that name a program are read, not shell text inside
  one;
- a `cp`, `mv` or `ln` destination that is an existing directory not named as one
  (`cp clippy.toml crates/foo`, with no trailing `/`);
- a write into a protected directory from a working directory inside it that an earlier Bash
  call moved to (`..` that climbs from inside `hooks/`).

These are left undecided and go to Clef D1 (below).

## Clef D1: `destructive_or_irreversible` (#30)

The denylist only denies a command or leaves it undecided (`denylist::Verdict`). An undecided
command gets `guard::Fallback`'s answer (`Allow`), and then, for a Bash call, Clef D1 is asked
`noul destructive_or_irreversible` (`hook::guard::clef`). A denylist denial short-circuits:
Clef is never asked about a command the denylist blocks, and other tools are never asked about.

D1 runs in process through `decide`'s library API (`decide::in_process` reads the thresholds,
configuration and anneal home as `anneal decide` does, then `decide::client::ask_recording`),
so it shares the command's thresholds (`$CLAUDE_PLUGIN_ROOT/playbooks/clef-thresholds.json`,
else the copy built in), modes, 50-decision shadow period and decision log (`docs/decide.md`).
**Time budget.** The guard plans to use 4.5 s of the hook's 5 s `timeout`, measured from when
it starts reading its input (the other 500 ms cover process start-up and writing the decision).
Reading the input, lexing and the denylist take milliseconds; the Clef call takes at most 2 s
(`decide`'s timeout), plus local reads and writes of the thresholds file, `counts.json` and the
log. A deny then records `gate_blocked` with only what is left: one lock wait of 500 ms for the
ledger, and git gets the rest up to its usual 1 s. With less than 200 ms left for git the record
is skipped and the deny's reason ends with `(the block was not recorded in the ledger: no time
left in the hook's budget)`; the deny itself always goes out. The fixed sum (2 s for Clef, 1 s
for git, 500 ms of lock wait) is 3.5 s, but it leaves out the local reads and writes around the Clef call and the fact that
the lock wait applies per ledger statement, which is why git's share is cut to what the clock
says is left rather than assumed. A lock held past the first statement can still cost a further
500 ms, inside the remaining margin.

| D1 for the call | Guard's answer |
|---|---|
| off: no account (`cf_account_id` empty), or `clef_mode` gives D1 `off` | as without D1: nothing is asked or logged |
| a malformed account, token or mode | as without D1: nothing is asked or logged (the mode is unknown, so it never acts) |
| shadow (the default, and `enforce` during its first 50 answered decisions) | as without D1, whatever Clef answers or fails to; one row logs the would-be action |
| enforce, `yes` probability 0.90 or more (`deny`) | deny, rule `destructive` (`ERROR guard destructive: Clef D1 rates this command ...`), plus `gate_blocked` with point `guard.destructive` |
| enforce, 0.40 or more (`ask`) | an ask decision (`"permissionDecision": "ask"`): the human confirms the call; no `gate_blocked` |
| enforce, below 0.40 (`defer`) | no output: Claude Code's normal permission flow decides |
| enforce, no answer (unreachable, timed out, bad response) | the point's fallback when it is `ask` (D1 ships with `ask`): an ask decision naming the failure; any other fallback allows, so a Clef failure never denies |
| enforce, an answer whose row or count cannot be written | the band's action still stands (a deny or ask notes `the decision was not logged`), and a `defer` asks instead: a failure never lets the call through unchecked |
| D1 cannot run (the anneal home, the plugin's thresholds file, `counts.json`, the log or the clock fails), configured `enforce` | an ask decision naming the failure (`Clef D1 could not run`) |
| D1 cannot run, any other mode | as without D1 (nothing to report it in: an allow has no output; `anneal decide` with the same environment shows the error) |

**Nothing changes for users yet.** The only transport is `NoHttpClient` (no HTTP client is a
dependency yet, `docs/decide.md`), so a configured Clef is always unreachable; unanswered calls
do not count toward the shadow period, so D1 stays in shadow and the guard's verdict is the
denylist's. Enforcing D1 needs the HTTP client and 50 answered decisions.

**Redaction** (`hook::guard::redact`). The command Clef gets (state
`{"tool": "Bash", "input": {"command": ...}}`) and the row keeps (its `input` field) is
redacted, then trimmed and cut to 1,024 characters (marked `…`). The rule does not depend on
where a word stands, so no wrapper (`sudo`, `nohup`, `timeout`, `env -u NAME`, `command export`)
or nested shell (`bash -c '…'`, `eval "…"`) can carry a secret past it; it redacts too much rather
than too little:

- the value of every `NAME=value` anywhere in the command (also `NAME+=`, `NAME[i]=` and the
  array form `NAME=(…)`) when `NAME`, with any quotes in it stripped (`export "API_KEY"=x`,
  `declare -x 'PASSWORD'=x`), looks like an environment variable (`[A-Z_][A-Z0-9_]*`) or
  holds a credential word in any letter case (`token`, `password`, `passwd`, `pass`, `secret`,
  `apikey`, `api-key`, `api_key`, `credential`, `private-key`, `private_key`, `auth`,
  `bearer`, `cookie`; so `api_token=x` and `db_pass=x` too, but not `author` as a whole word,
  so `--author=` stays readable). Lower-case operands such as `dd of=/dev/sda bs=1M` stay
  readable, since they are what Clef judges; `make CC=gcc` is redacted. A quote opened anywhere
  in a value (`TOKEN=abc"def ghi"`) is part of it, through its close;
- the value after a credential-named key anywhere in a word, with any quotes between the key
  and its `=` or `:` (`--from-literal=password=x`, `-Dpassword=x`,
  `//registry/:_authToken=x`, `{"password":"x"}`, `pass:x`): the rest of that word. A `::`
  is a path, not a key (`auth::tests` stays);
- a header: a credential-named key ending its word in `:` (`"X-Api-Key: …"`,
  `PRIVATE-TOKEN:`, `Cookie:`, `{"Authorization": …}`), any `Authorization:` (any letter
  case, `Proxy-Authorization:` too), hides its own value and every word after it up to the
  end of the simple command or quote it is in: `-H "Authorization: Bearer x"` becomes
  `-H "Authorization: <redacted> <redacted>"`. A header-shaped credential name with its value
  attached (`Cookie`, or a name with a `-` inside that ends in a credential word:
  `Cookie:a=1`, `X-Api-Key:k`) hides only that value, since such a name may be no header at
  all: `docker run -v db-password:/p alpine rm -rf /p` keeps `alpine rm -rf /p`, and
  `git push origin fix-auth:main --force` keeps `--force`. Every separator (`;`, `&`, `|`, a
  newline) ends a header's value, so it never hides the next command, in a `bash -c '…'`,
  `eval "…"`, `$( )` or backtick body too; only inside a quote or substitution, the
  `name=value` words right after a `;` that ends either form stay hidden
  (`"Cookie: a=1; b=2"` and `"Cookie:a=1; b=2"` become `"Cookie: <redacted>; <redacted>"`
  and `"Cookie:<redacted>; <redacted>"`), read through the punctuation around them, so a
  quoted value (`sid="S"`) or a quote closing the header's own (`'Cookie: a=1; sid=S'` inside
  `bash -c "…"`) counts too, but not a word holding a substitution or escape (`b=$(…)`) or
  whose value holds a blank or a `;` or opens a quote it does not close (`c="rm -rf ~"`),
  which could run a command. The first word after a `;` that is not such a pair is read as
  usual, and so stays readable, as does every word after it: `"Cookie: a=1; Secure; b=2"`
  keeps `Secure; b=2`. The words after the header in its own simple command stay hidden;
- the value of every option, one dash or two, whose name holds a credential word, attached or
  as the next word (`--token=x`, `--password x`, `-storepass x`, `-passout pass:x`), except
  one whose name ends in `-stdin`, `-file` or `-env`, whose value is not on the command line
  (`docker login --password-stdin -u me` keeps `-u me`);
- the word after a configuration key naming a credential, a whole word of letters, digits,
  `-`, `_` and `.` that holds a `.` or `_` and is not an option (`aws configure set
  aws_secret_access_key x`, `git config user.password x`), with the same exceptions; a file
  named like one (`cat id_token.txt out`) costs its next word;
- the word after `Bearer`, and the word after `Basic` unless it is plain lower-case letters
  (`basic usage` stays);
- the userinfo of every URL, up to the last `@` before the path or query, so a `#` in a
  password is covered (`https://user:p#ss@host` becomes `https://<redacted>@host`), and so is
  a `?` when a `:` comes before it and an `@` after it (`https://user:p?X@host`), while
  `https://host?to=a@b` stays; `git@host:repo` is not a URL and stays. A port before the
  query reads like a password, so `https://host:8080?to=a@b` becomes `https://<redacted>@b`, an
  accepted over-redaction;
- an unquoted word that looks like a token: a known prefix (`sk-`, `ghp_`, `gho_`, `ghu_`,
  `ghs_`, `ghr_`, `github_pat_`, `xox[abps]-`, `glpat-`, Vault's `hvs.` and `hvb.`, `eyJ`),
  an AWS access key id, or 32 or more letters and digits mixed (a full commit hash too);
- the word after `vault login`, whatever it is (a legacy `s.` token has no prefix worth
  matching alone; `vault login -method=userpass` costs that option);
- the options whose value is a secret though their name names none, for the tools known to
  take them (`decide::redact::tools`), from the tool's name (its last path component, quotes
  and backslashes removed, so `sudo /usr/bin/mysql`, `"mysql"` and `\mysql` count) to the end
  of its simple command, the option read with its quotes removed (`'-pX'`): `-p` attached for
  `mysql`, `mysqldump` and the other MySQL and MariaDB clients (`-pX`; `-p` alone prompts and
  `-p db` names a database); `-u`, `-U` and `-b` (cookie) for `curl`, attached, in a cluster
  (`-su x`) or as the next word, and `--user` and `--proxy-user`; `-p` for `sshpass` and the
  MongoDB tools; `-a` for `redis-cli`; `-P` for `sqlcmd` and `bcp`; `-w` for the OpenLDAP
  clients; `-U` (`user%pass`) for `smbclient`, `rpcclient` and `smbcacls`; `-p` after `login`
  for `docker`, `podman`, `nerdctl` and `helm`; `-p` for `twine`, and after `login` for `az`;
  `-b` and `--body` after `secret` for `gh`.
  Other tools' short options stay readable (`mkdir -p`, `docker run -p 8080:80`);
- the inside of quotes and substitutions, scanned as text: `'…'`, `$'…'`, `$( )` and backticks
  as a command, `"…"` with double-quote rules (there `'` and `$'` are plain characters);
- an unquoted `#` comment, to the end of its line, and every heredoc body (its closing line is
  kept);
- from any quote, substitution or heredoc that is never closed (or nested past 32 levels), the
  whole rest of the command.

The guard's lexer keeps no source positions, so a small scanner with the same quoting rules
finds each span and rewrites only what it redacts. A word's secret is taken where the readable
part before it closes every quote it opens; otherwise the inside of the quote is scanned and
finds it there. The word rules live in `decide::redact`, shared with the prompt redaction
(`docs/hooks.md`). Known gaps: an option whose value is a secret but whose name names no
credential, for a tool not listed above (`sometool -k s3cret`); a header whose name names no
credential (`-H "X-Signature: …"`); a secret that is just a word in a non-credential position
(`echo hunter2 | login`), which includes the tools that take a password as a positional
argument (`htpasswd -b file user pass`, `cf auth user pass`); a header value's word after a `;`
that is not a `name=value` pair, and every word after it (`"Cookie: a=1; Secure; sid=S"` keeps
`sid=S`); and a secret inside a file the command reads.

**Outcome labels: not yet.** The blueprint labels a D1 decision from the human's approve or deny
on an ask, and a revert within 24 h as destructive. No deterministic signal for either exists in
this codebase: no hook sees a Bash call after it ran (a `PostToolUse` hook on Bash would: an
asked command whose `tool_use_id` later shows up there was approved, one that never does was
denied or abandoned), and no hook watches reverts. In shadow mode nothing is asked, so there is
nothing to label until D1 is enforced. Labels come from `anneal decide label` for now; automatic
labelling is a follow-up.

## Adding a denylist entry

Adding a denylist entry means a module under `hook/guard/denylist/`, an arm in `Checker::program` (`denylist/checker.rs`), a
`guard::Rule` with an id, a table test with near misses, and a row in `BASES` of
`denylist/tests/forms.rs`, which checks it in every wrapping form.
