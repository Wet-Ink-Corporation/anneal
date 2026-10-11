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
| the command cannot be read: an unclosed quote, `$(`, backtick or `${`, a `case` with no `esac` inside `$( )`, a redirection with no target, nesting deeper than 32 | deny, rule `unparseable` |
| `bash -c`, `eval`, `env -S` or `find -exec` nested deeper than 8, or more than 4 MiB lexed in total across those levels | deny, rule `unparseable` |
| a form the guard does not model (see [Bypass handling](#bypass-handling)): a script from stdin or a process substitution known only at run time, a program word built by brace expansion or from a command's output, a `git -c alias.*` | deny, rule `unparseable` |
| brace expansion that makes more than 256 words | deny, rule `unparseable` |
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
`-fu`), `--force-with-lease[=...]`, `--mirror`, or a refspec that starts with `+`. A forced
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
`-r -f`, `--recursive --force`, flags after the operands). It is allowed only when every
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

`git add` (or its synonym `git stage`) with `-f`, `--force`, or `f` in a short-option cluster
(`-fA`). It stages files that `.gitignore` excludes. Allowed: `git add -- -f` (a path named
`-f`) and every other `git add`.

### `config-write`: Bash writes to lint or hook config

A Bash command may not change these paths. They are matched on path components, so
`docs/hooks.md` and `myhooks/x` do not match:

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
- `sed -i` (also `-i.bak`, `--in-place`) operands. The script is not a path, so
  `sed -i 's/hooks/x/' notes` is allowed;
- the destination of `cp`, `mv`, `ln` and `install`, including `-t DIR`. When the destination
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
| `case` | a pattern's `)` does not close an enclosing `$( )` or `${x:-$( )}`; `case` and `esac` are counted in command position |
| Substitutions: `$( )`, backticks, `<( )`, `>( )`, `$(( ))`, `${x:-$(...)}` | the inner commands are checked; the word holding one is dynamic; a process substitution is a word (a pipe's path) |
| Heredocs | the body is data and the reading command's stdin; with an unquoted delimiter its substitutions are checked |
| `bash -c`, `sh -c`, `zsh -c`, `dash`, `ksh`, `mksh`, `ash` (also `-lc`, `-o opt -c`) | the string is lexed and checked, up to 8 levels deep and 4 MiB in total |
| A shell reading its script from stdin: `bash <<'EOF'`, `sh -s`, `bash <<< '...'`, `echo '...' \| sh`, `bash /dev/stdin`, `sudo -s`, `su`, `script` | the script is checked when it is static: a heredoc whose delimiter is quoted (or whose body has no substitution), a literal here-string, or what `echo` or `printf` with literal arguments (no `\` escape, no `%` directive) pipes in. Any other stdin (`curl ... \| sh`, `cat x \| bash`, `bash <<< "$S"`) is denied |
| A script from a process substitution or descriptor: `bash <(curl ...)`, `source <(...)`, `bash /dev/fd/3`, `bash < <(...)` | denied |
| `eval`, `alias NAME=VALUE`, `trap '...' SIG`, `su -c`, `script -c`, `watch`, `flock -c`, `env -S '...'` | the string is lexed and checked (`eval` and `watch` join their arguments) |
| Environment: `VAR=x cmd`, `env [-i] [-u N] VAR=x cmd` | assignments and options are skipped |
| Wrappers: `sudo`, `doas`, `command`, `builtin`, `exec`, `nohup`, `time`, `nice`, `timeout`, `setsid`, `stdbuf`, `ionice`, `chrt`, `taskset`, `unbuffer`, `busybox`, `flock`, `xargs`, `find -exec` | their options (and the duration, priority, mask or lock file) are skipped and the wrapped command is checked; every `-exec`, `-execdir`, `-ok` and `-okdir` of a `find` is checked |
| Program paths: `/bin/rm`, `/usr/bin/git`, `git.exe` | only the last path component, without `.exe`, is used |
| Brace expansion: `git push {-f,origin,main}`, `tee .claude/{settings.json,x}` | arguments are expanded (up to 256 words) and the words keep their pattern shape; a program word built by brace expansion (`{rm,-rf,/}`) is denied |
| Variables set earlier in the command: `F='rm -rf /'; $F`, `T=.claude/settings.json; echo x > $T` | a standalone assignment (also after `export`, `declare`, `local`, `typeset`, `readonly`) with a literal value is put in where the variable is used, split into words when unquoted. A variable the command sets to a value known only at run time (`F=$(...)`, `read F`, `for F in`, `printf -v F`, `F+=x`, two different values) and then runs as the program is denied; after the command sets `IFS`, unquoted values are not put in |
| A program word that is only expansions: `$(echo rm) -rf /`, `"$GIT" push -f`, `rm${IFS}-rf${IFS}/` | its literal parts are read as words (each expansion a separator), and its arguments are checked against every entry. If it is a command's output (`$(...)`, backticks) or the arguments of a function or script (`"$@"`, `$*`, `$1`), it is denied |

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
- a `cp`, `mv` or `ln` destination that is an existing directory not named as one
  (`cp clippy.toml crates/foo`, with no trailing `/`);
- a write into a protected directory from a working directory inside it that an earlier Bash
  call moved to (`..` that climbs from inside `hooks/`).

These are left undecided and go to the extension point.

## Extension point: Clef D1 (#30)

The denylist only denies a command or leaves it undecided (`denylist::Verdict`). Every
undecided command goes to `guard::Fallback`, which today has one variant, `Allow`. Clef D1
plugs in as a new `Fallback` variant whose `decide` asks Clef, within the 2 s budget for Clef
calls, and maps the answer to a `Decision` (allow or deny with a reason). The match is
exhaustive, so a new variant has to say what it does. Adding a denylist entry means a module
under `hook/guard/denylist/`, an arm in `Checker::program` (`denylist/checker.rs`), a
`guard::Rule` with an id, a table test with near misses, and a row in `BASES` of
`denylist/tests/forms.rs`, which checks it in every wrapping form.
