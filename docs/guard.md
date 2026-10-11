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
  `(the block was not recorded in the ledger: ...)`.

### Failing closed

Whatever the guard cannot judge, it denies:

| Failure | Result |
|---|---|
| stdin is not `PreToolUse` JSON, is larger than 1 MiB, or cannot be read | deny, rule `bad-input` |
| a Bash call with no `tool_input.command` | deny, rule `bad-input` |
| the command cannot be read: an unclosed quote, `$(`, backtick or `${`, a redirection with no target, nesting deeper than 32 | deny, rule `unparseable` |
| `bash -c`, `eval`, `env -S` or `find -exec` nested deeper than 8 | deny, rule `unparseable` |
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
`crates/anneal-cli/src/hook/guard/denylist/tests.rs` hold every row below and more.

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
skipped before the subcommand.

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
or `find -exec` adds cannot be seen, so `rm -rf` under them is denied.

Allowed: `rm -rf /tmp/build`, `rm -r src` (not forced), `rm -f file` (not recursive), and
`rm`, `-rf` or `/` in a quoted string (`echo 'rm -rf /'`).

### `git-add-force`: `git add -f`

`git add` with `-f`, `--force`, or `f` in a short-option cluster (`-fA`). It stages files that
`.gitignore` excludes. Allowed: `git add -- -f` (a path named `-f`) and every other `git add`.

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

## Bypass handling

The command goes through a POSIX-ish lexer (`hook/guard/shell.rs`), and then every simple
command it runs is unwrapped (`hook/guard/denylist/wrappers.rs`):

| Form | Handling |
|---|---|
| Quoting: `"git" push`, `g'i't`, `r\m`, `rm -r''f`, `$'\x72\x6d'` | quotes and escapes are removed before matching; ANSI-C escapes are decoded |
| Chains: `&&`, `\|\|`, `;`, `\|`, `\|&`, `&`, newlines, `\`-newline | each simple command is checked |
| Subshells and groups: `( )`, `{ ...; }`, `if`/`then`/`while`/`do`/`!` | flattened; leading keywords are skipped |
| Substitutions: `$( )`, backticks, `<( )`, `>( )`, `$(( ))`, `${x:-$(...)}` | the inner commands are checked; the word holding one is dynamic |
| Heredocs | the body is data; with an unquoted delimiter its substitutions are checked |
| `bash -c`, `sh -c`, `zsh -c`, `dash`, `ksh`, `mksh`, `ash` (also `-lc`, `-o opt -c`) | the string is lexed and checked, up to 8 levels deep |
| `eval` | its arguments are joined and checked, as `eval` does |
| Environment: `VAR=x cmd`, `env [-i] [-u N] VAR=x cmd`, `env -S '...'` | assignments and options are skipped; `-S` is lexed and checked |
| Wrappers: `sudo`, `doas`, `command`, `builtin`, `exec`, `nohup`, `time`, `nice`, `timeout`, `xargs`, `find -exec` | their options are skipped and the wrapped command is checked |
| Program paths: `/bin/rm`, `/usr/bin/git`, `git.exe` | only the last path component, without `.exe`, is used |
| A program only known at run time: `$(echo rm) -rf /`, `"$GIT" push -f` | checked against every entry |

### Limits

The guard reads the command. It does not run it, so it cannot know what the shell will do with
values that exist only at run time:

- variables and substitutions as operands (`rm -rf "$DIR"` is denied, but `cp x "$DEST"` is
  allowed and `git push -f origin "$B"` is denied);
- scripts the command runs (`bash script.sh`, `source x`, `echo '...' | sh`, `python -c`);
- symbolic links (`/tmp/link` that points elsewhere);
- a `cp`, `mv` or `ln` destination that is an existing directory not named as one
  (`cp clippy.toml crates/foo`, with no trailing `/`).

These are left undecided and go to the extension point.

## Extension point: Clef D1 (#30)

The denylist only denies a command or leaves it undecided (`denylist::Verdict`). Every
undecided command goes to `guard::Fallback`, which today has one variant, `Allow`. Clef D1
plugs in as a new `Fallback` variant whose `decide` asks Clef, within the 2 s budget for Clef
calls, and maps the answer to a `Decision` (allow or deny with a reason). The match is
exhaustive, so a new variant has to say what it does. Adding a denylist entry means a module
under `hook/guard/denylist/`, an arm in `denylist::check_program`, a `guard::Rule` with an id,
and a table test with near misses.
