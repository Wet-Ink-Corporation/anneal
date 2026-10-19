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
  statement waits at most 100 ms for another process's lock (`a_git_that_never_answers_still_gets_the_deny_out_within_the_hook_budget`).

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
| no anneal binary is installed | the launcher exits 0 and the guard is off; the session start warns (`docs/hooks.md`) |
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

An operand that may expand to an option cluster may hold the flags: one whose first part may
be a `-` (a variable or substitution, which may hold one; an unquoted `?`, `*` or bracket
expression, which may match a file named `-rf`; or a written `-`) and whose other written
characters are only letters, digits and `-` (`$F`, `"$FLAGS"`, `"$a$b"`, `"$b"-rf`, `-$x`,
`"$b"rf` with `b=-`, `?rf`, `[-]rf`). An extended glob group (`shopt -s extglob`: `@( )`,
`+( )`, `!( )`, `?( )`, `*( )`) may match any text, so `rm @(-rf) /home`, `rm +(-)rf /home`
and `rm !(x) /home` are denied while `rm -f @(a|b).o x` is allowed; a `(` or `)` inside a
quote does not close the group. A protected-config path reads a component's text from its
first group to its last `)` as one or more `*` components, so a quoted `)` cannot cut a group
short there either (`tee .c@(laude|l")")/settings.json` is denied), and a `/` inside a group
is no separator (`tee .clau@(de|x/y)/settings.json` is denied). A special parameter's name
before `(` opens a group too, as bash reads it (`$@(x)`, `$?(x)`). The lexer reads such a
group as one pattern word, blanks and `|` included (bash without `extglob` refuses the word,
so nothing runs then), and checks the commands of a `$( )` substitution inside it. A `<( )`
or `>( )` inside a group (a line continuation between `<` and `(` included) is
`unparseable`: bash ends the group at the first `)` that balances it, which may be a `case`
pattern's inside the substitution, so what runs after it cannot be read reliably
(`echo @(<(ls))` is denied). A backtick inside a group, at any depth, is `unparseable` too:
inside a backtick substitution bash ends the substitution there, not where the group ends
(`` echo `echo @(a`:`rm -rf ~ # )` `` runs `rm`), so `` echo @(`ls`) `` is denied as well.
A `$[ ]` inside a group is `unparseable` too, since bash skips it whole, parentheses
included; a line continuation after a `$` inside a group is dropped first, as bash drops it
(`echo @($\<newline>(rm -rf /))` is judged as `rm -rf /`).
An `env -S` string is split by env, which reads no group, so no group is read there:
`env -S '@(rm -rf ~)'` is judged as `rm -rf ~`, as before extended globs were read. The string
is also read as shell text, since a program it runs may be a shell or `watch`, and the words
after it are the shell's, which reads their groups (`env -S 'watch rm @(-rf) /home'` and
`env -S rm @(-rf) /home` are denied); either reading's denial denies. It is also read as env
splits it, with each word after the string kept whole (`env -S bash -c 'rm -rf /'` is denied)
and env's escapes applied, as GNU coreutils 9.4 splits it: `\_`, and outside quotes a LF, CR, VT
or FF, separate words, and `;`, `&`, `|`, `<`, `>`, `(` and `)` are characters of their word (env
reads no operator, so `env -S 'rm -rf ; /home'` is denied); outside single quotes a backtick is a
character of its word too, as env runs no substitution; `\t`, `\n`, `\r`, `\f` and `\v` are those characters inside their word;
inside single quotes `\'` is a quote and `\\` a backslash; and `\c` or an unquoted `#` starting
a word ends the string, so the words after it follow (`env -S 'bash -c #' 'rm -rf /'`,
`env -S 'rm\_-rf\_/'` and `env -S 'sh -c rm\t-rf\t/'` are denied). env then reads the words it
split as its own arguments, so they are read as env's again (`env -S '-S "rm -rf /"'` and
`env -S '-i -S rm\_-rf\_/'` are denied). A `${VAR}` env expands is read as a variable.
A `!(` in command position (after only reserved words, `time`'s `-p` or `--`, or
`function NAME {` and `coproc [NAME] {`) stays as bash reads it without `extglob`: the `!`
keyword and a subshell whose commands are checked. A word ending in an opener before an
empty `()` (blanks and line continuations allowed) where a function's name stands (command
position, or after `function`) is `unparseable`: without `extglob` it defines a function,
with it it is one pattern word whose later words are its arguments (`/bin/echo@() { hi }`
runs `echo`), so `f?() { ...; }` and `/bin/rm@() -rf ~` are denied (a cost: name such a
function without the opener). Elsewhere `@()` is an empty group, so `rm @()-rf /home` is
denied. A group in the program word may name any program, as `?` and `*` may:
`/bin/r@(m) -rf ~` is judged as an unknown program, its arguments checked.
A bracket expression may match `-` unless it is a plain
set that names none: `[!a]` and `[[:punct:]]` match it too, and so may `[$c]` (a `]` inside an
expansion in the set, as in `[!$(echo ])]`, does not close it). A `[:` or `[=` that never
closes is read as bash reads it: its `[` is a member and the set closes at the next `]`, a set
taken to match `-` (`rm [[:-]rf /home` is denied, and so, as a cost, is `rm -f [[:a]x /home`);
a `[.` that never closes is read the same way, though bash 5 matches nothing there.
Each operand is judged as one
word, its expansions as quoted text: the words an unquoted expansion or `"$@"` may split into
are not (see [Limits](#limits)). When another
operand follows it, it is taken as `-rf`, so `F=-rf; rm $F /`, `rm "$FLAGS" build`,
`rm "$b"-rf /`, `rm "$b"rf /home/x`, `rm "$a"{r,f} /home` and `rm ?rf /` are denied while
`rm -f "$tmp"`, `rm "$f".txt "$g".txt`, `rm -f "$tmpdir"/x "$tmpdir"/y`, `rm -f *.o *.a` and
`rm -f [.]rf x` are allowed. An operand after `--` is never an option, so it never holds the
flags: `rm -f -- "$p"-old x` and `rm -- ?rf /tmp/x` are allowed. A cost: `rm -f "$a" "$b"` is
denied too, since `$a` may be `-r`, and so are `rm -f "$p"-old "$p"-new` and
`rm "$p"old "$p"new` (`"$p"old` is `-rold` when `p` is `-`) and `rm -f * x` (`*` may match a
file named `-r`); write `--` before such operands.

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
| `.anneal/config.json` | `config.json` directly in `.anneal`: the gates' own settings (the orchestrate leads the brief hook trusts, the merge gate's `review` and `ship` keys, `check_fast`); the preconditions hook denies the Edit, Write, MultiEdit and NotebookEdit tools on it |

The writes it checks:

- a redirection: `>`, `>>`, `>|`, `&>`, `&>>`, `<>`, `>&file`, with or without a descriptor
  number;
- `tee` and `truncate` operands;
- `sed -i` (also `-i.bak`, `--in-place`, `--in`) operands. The script is not a path, so
  `sed -i 's/hooks/x/' notes` is allowed;
- the destination of `cp`, `mv`, `ln` and `install`, including `-t DIR` and
  `--target-directory` (or `--target`, `--t`). When the destination
  is a directory (`-t`, a trailing `/`, or `.anneal`, `.claude`, `.github`, `.cargo`), the destination is
  also checked joined with each source's name, so `cp settings.json .claude/` is denied;
- what `mv` moves away, and what `rm`, `unlink`, `rmdir`, `shred`, `chmod`, `chown` and `chgrp`
  touch. These also deny `.anneal`, `.claude`, `.github` or `.cargo` itself, because removing or
  re-permissioning that directory changes the config inside it;
- the sources of a hard link: `ln` without `-s`/`--symbolic` (also `ln FILE` alone, which links
  into `.`), `link`, and `cp -l`/`--link`. Writing a hard link writes the file it links, so
  `ln .anneal/config.json x` is a write of `.anneal/config.json`, as are a source directory that
  holds config (`cp -al .claude /tmp/c`) and the working directory or `/` as a source. A
  symbolic link is not denied: a later write through it is a gap (below).

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
to a variable or to `-`, or a `popd`, relative paths may start anywhere, so they are matched as
climbing above their start (`cd "$DIR" && echo x > settings.json` is denied). A `cd` is followed
once, as written: its directory is not read again with its variables empty, which would place
`cd "$d"/x*` at `/x*` rather than anywhere (`cd "$d"/x* && echo x > settings.json` is denied).

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
| Wrappers: `sudo`, `doas`, `command`, `builtin`, `exec`, `nohup`, `time`, `nice`, `timeout`, `setsid`, `stdbuf`, `ionice`, `chrt`, `taskset`, `unbuffer`, `busybox`, `flock`, `xargs`, `find -exec` | their options (and the duration, priority, mask or lock file) are skipped and the wrapped command is checked; every `-exec`, `-execdir`, `-ok` and `-okdir` of a `find` is checked. Options are read as each wrapper's getopt reads them (#121): short options cluster (`sudo -iu root` is `-i -u root`), a value letter takes the rest of its cluster or else the next word (`-uroot`, `-u root`), an optional letter only the rest of its cluster (`xargs -eE`, `watch -dn`), long options take their value after `=` or in the next word and may be abbreviated. Where implementations differ the guard takes their union: `sudo` with `doas` (`-a`, `-c`, `-R` take values), GNU with BSD `env` (`-L`, `-P`, `-U`) and `xargs` (`-J`, `-R`, `-S`). `chrt` reads a priority only when it is all digits, then skips one `--` (util-linux 2.42; `chrt -o rm -rf /` runs `rm`); with `-p` the words after the priority are still read as a command, since chrt runs them with a pid of 0. `su` and `script` read options anywhere, and every `-c` (`--command`, su's `--session-command`) is read, each on its own, the last being the one they run. `su` also hands its words after the user (its `-` and user may follow `--`: `su - -- root -c 'rm -rf /'`; all of them under `POSIXLY_CORRECT`) to the user's shell, read as that shell reads its arguments: its options, then the command string of `-c` (`su root -- -c 'rm -rf /'`, `su root -- -co pipefail 'rm -rf /'`), or else its first operand as a script file, so `su root -w PATH <(curl ...)` and `su root -g root +x <(curl ...)` run a process substitution and are denied; the words after `--` are also read on their own. `sudo -h` takes the next word as a host, since sudo runs the command when it names the local host. The tables also hold options newer than the probed binaries (coreutils `env -a`, procps-ng `watch -s`, util-linux `chrt -U`/`-X`), and FreeBSD `env` reads `--C` as `-C`; a later version that adds a value option is a gap until its table learns it. Each grammar is checked against the installed binaries by `crates/anneal-cli/tests/fixtures/wrappers/probes.txt`. A glob or extended glob where a shell or wrapper reads an option may expand to one when a file of that name exists (`bash -[c] 'rm -rf /'` runs `bash -c` beside a file named `-c`), so such a word is read both as the walk reads it, an operand or a value, and as an option the guard cannot know, which denies (#165): a word holding a glob or group whose first part may be the option's sign and whose second, if any, may be an option letter, whatever value is attached (`-[c]`, `-@(c)`, `?c`, `@(--)login`, `env -S@(rm -rf ~)`, `env -[S]'rm -rf ${HOME}'`), and, where options take a value, any word led by `*`, an expansion or a group, which may hold the sign, the letter and the value's start at once (`env *' rm -rf ~'`); a shell's options may be signed `+` too (`bash +[c] '...'`, `bash [+]c '...'`). The slots are the words a wrapper takes before the command it wraps and that command's first word; for a shell, the words it reads as options or their values (each `o`, `O` and `--rcfile` owing one, in both the long-option and the short-only reading) and the word that ends them, its command string or script (`sh -c '...' *` and `bash -e run.sh *` are allowed); for `sudo -s`, `env -S`, `watch` and `flock -c`, the words their own grammar reads as options and the word after them; for `su`, every word, the words after `--` included, since it hands them to the user's shell; for `script`, every word before a `--`. A pattern that cannot start an option (`bash *.sh`) or stands after the slots (`bash run.sh *`, `sudo rm -f *.o`) is read as before; `find`, `eval`, `source`, `trap`, `alias`, `function`, `coproc` and `busybox` read no options of their own here |
| Unlisted wrappers: `caffeinate`, `runuser -u root --`, `chroot /`, `unshare`, `nsenter`, `systemd-run`, `pkexec`, `parallel ... :::` | the arguments of any program the guard does not know are read as a command from each word that names a checked program (`rm`, `git`, `tee`, `truncate`, `cp`, `mv`, `install`, `ln`, `link`, `sed`, `unlink`, `rmdir`, `shred`, `chmod`, `chown`, `chgrp`), a shell or a wrapper above. Programs that only print, search or test their arguments (`echo`, `printf`, `grep`, `rg`, `man`, `which`, `type`, `test`, `[`, `for`, `case`, ...) are skipped. The cost is some false denials: `cargo run -- rm -rf build` or `ssh host git push -f origin main` are denied |
| Long options | an abbreviation counts as the option (`--forc` is `--force`, `--comm` is `--command`), for the entries and for the wrappers that use `getopt_long`; the shells' own long options must be spelled out, as bash requires, with one dash or two (`-rcfile`, `--norc`) and only before the first short option; every shell, bash included, is also read without long options (a name does not say which shell runs: `busybox bash` is ash), so a body after a long option is hidden from Clef (`bash --login -c <redacted>`) and `git log | bash --version` is denied, as dash (`sh` on Debian) and bash after su's `-c` or `-f` read them, where `-posix` is a cluster; in a shell's short options each `o` or `O` takes the next word (`-ox pipefail`) and `+c`, `+s` count as `-c`, `-s` |
| Program paths: `/bin/rm`, `/usr/bin/git`, `git.exe` | only the last path component, without `.exe`, is used |
| Brace expansion: `git push {-f,origin,main}`, `tee .claude/{settings.json,x}` | a command holding a brace group is read brace-expanded (up to 256 words), as the shell expands it before any program, wrapper or shell reads its options. As in bash, a quoted or escaped `{`, `,` or `}` is no part of a group (`bash {-c,'echo ,; rm -rf /'}` is `bash -c 'echo ,; rm -rf /'`), nor is a variable's value or xargs input, and an empty product nothing quoted went into is no word (`echo 'rm -rf /' \| bash {,}` runs `bash`, which reads the script from stdin; `''{,}` is two empty words). A word with no other pattern character and no variable or substitution left in it (whatever the other words of its group kept: `git push {-f,origin$x}` is a force push) reads as written where a program name, option or gh operand is read (`bash {-c,'rm -rf /'}` is `bash -c 'rm -rf /'`, `caffeinate {sudo,-u,root,rm,-rf,/}` is `sudo -u root rm -rf /`), and keeps its pattern shape for the entries (`rm -rf /tmp/{a,b}` is denied). A program word that holds a group in the command as written, its wrappers removed (`{rm,-rf,/}`, `sudo {sh,-c,'rm -rf /'}`, `sudo {-u,root} ls`), is denied. Among the arguments of a program the guard does not know, a checked program, shell or wrapper is read as a command: `caffeinate {rm,-rf,/}`, `caffeinate {bash,-c,'rm -rf /'}` and `caffeinate "$b"{rm,x} -rf /` are denied as `rm -rf /`, while `brew install {git,jq}` and `make {install,clean}` are allowed as their literal forms are |
| A brace or glob after a variable or substitution: `rm "$b"{-rf,/}`, `git push "$b"{-f,origin,main}`, `tee "$d".clau*/settings.json`, `echo x > "$d".clau*/settings.json` | a word with an unquoted `*`, `?`, `[`, `{` or leading `~` is brace-expanded whatever else it holds (the braces of `${x}` and the text of `$( )` are never a group), and such a word is judged in two readings: as written, and with its expansions empty, as they are when the variable is empty. A command's arguments are checked in both, and so is a redirection's target; either denial denies. A word with an expansion but no unquoted pattern character (`tee "$d".claude/settings.json`) is judged as written only |
| Variables set earlier in the command: `F='rm -rf /'; $F`, `T=.claude/settings.json; echo x > $T` | a standalone assignment (also after `export`, `declare`, `local`, `typeset`, `readonly`) with a literal value is put in where the variable is used, split into words when unquoted. A variable the command sets to a value known only at run time (`F=$(...)`, `read F`, `for F in`, `printf -v F`, `F+=x`, two different values) and then runs as the program is denied, as is a name reference or array (`declare -n`, `local -n`, `-a`, `-A`, alone or in a cluster such as `-gn`); after the command sets `IFS`, unquoted values are not put in |
| A program word that is only expansions: `$(echo rm) -rf /`, `"$GIT" push -f`, `rm${IFS}-rf${IFS}/` | its literal parts are read as words (each expansion a separator), and its arguments are checked against every entry. If it is a command's output (`$(...)`, backticks), the arguments of a function or script (`"$@"`, `$*`, `$1`) or an indirect expansion (`${!y}`), it is denied |

### Limits

The guard reads the command; it does not run it. It is one layer of several, so it denies what
it cannot model instead of modelling every shell feature, and it does not try to see:

- values from the environment (`"$EDITOR" file`, `$GIT status` run, with their arguments
  checked; `cp x "$DEST"` is allowed and `git push -f origin "$B"` denied);
- word splitting (a documented limit, Ryan's decision on #128): an unquoted expansion,
  `"$@"` or `"${a[@]}"` may stand for zero, one or several words, and each word is judged as
  one. So a split can put `-rf` in an `rm` operand of its own (`rm x$b /home/x` with
  `b=' -rf'`, `rm $F` with `F='-rf /home'`, `set -- a -rf; rm x"$@" /home/x`,
  `f() { rm x"$@" /home/x; }; f a -rf`), and an expansion that is empty can vanish and move a
  later word into a checked place (`bash -c $x 'rm -rf /'`, `caffeinate bash $x -c 'rm -rf /'`,
  `cp x .claude/settings.json $x`, `bash {-xc,$x} 'rm -rf /'` with `x` unset). These are
  allowed, because judging every such word as possibly several would also deny everyday
  commands such as `rm -f $f` and `rm -f $dir/x /y`;
- a quoted expansion in `git push` that holds a force flag: `git push "$branch"` with
  `branch=--force` (or `-f`) is allowed, since denying it would deny every
  `git push origin "$branch"` (a documented gap, Ryan's decision on #127). A glob that may
  match a file named like the flag is the same gap (`git push ?f origin main`,
  `git push @(-f) origin main`). A variable set
  earlier in the same command is put in and judged (`b=-f; git push $b origin main` is denied);
- a `!(` in command position when `extglob` is on: bash reads it as a glob there, while the
  guard reads the `!` keyword and a subshell, as bash does without `extglob`. Where the two
  readings end the group at different `)` (a `#` comment inside it), a later command can be
  missed (main had the same gap, #144);
- a variable where a shell or wrapper reads an option (`bash $o 'rm -rf /'` with `o=-c`):
  word splitting is the documented limit above (#128), and only globs and extended globs
  there are judged as possible options (#165);
- a glob that matches nothing under `nullglob` vanishes and moves a later word into an option
  slot (`shopt -s nullglob; bash [z] -c 'rm -rf /'`), since a pattern that cannot start an
  option ends the slots (#170);
- a `find` action written as a pattern (`find . ?exec rm -rf / \;` beside a file named
  `-exec`): `find` has no option slots, so `find -name *.rs` stays allowed (#171);
- long options a shell has only in some builds: bash's `--wordexp` and `--protected` (with
  `WORDEXP_OPTION`) are read as short-option clusters, as other builds read them;
- scripts in files and the code of other interpreters (`bash script.sh`, `source x`, `make`,
  `npm run`, `python -c`, `node -e`, `perl -e`);
- other programs that delete or write files (`find -delete`, `git clean`, `dd of=`,
  `curl -o`), and environment variables that make git run a command (`GIT_SSH_COMMAND`,
  `GIT_EDITOR`);
- `PATH` changes, functions or aliases defined by an earlier Bash call, and how a pipe's
  writer that is not a static `echo` or `printf` reaches `xargs`;
- symbolic links (`/tmp/link` that points elsewhere), including one the command makes to
  protected config and then writes through (`ln -s .anneal/config.json x; echo > x`);
- a command string given to a program it does not know as one word (`ssh host 'rm -rf /'`,
  `tmux new 'git push -f'`): only words that name a program are read, not shell text inside
  one;
- a `cp`, `mv` or `ln` destination that is an existing directory not named as one
  (`cp clippy.toml crates/foo`, with no trailing `/`);
- a write into a protected directory from a working directory inside it that an earlier Bash
  call moved to (`..` that climbs from inside `hooks/`).

These are left undecided and go to Clef D1 (below).

The merge gate (`docs/verdict.md`) reads commands with this checker in a probe mode
(`Entries::PrMerge`) that collects `gh pr merge` calls and notes `GH_REPO`/`GH_HOST`
assignments and directory changes (`cd`, `pushd`, `popd` anywhere; `env -C`, `sudo -D`,
`sudo -i`, `su -`/`-l`, `find -execdir`). Those notes are defence in depth with the same
limits as above (a gh run from a git pager, a name built at run time; an `env` short cluster
such as `env -vu X -C` is read as env reads it since #121): the merge gate's invariant rests
on the merge naming its repository with `--repo <host>/<owner>/<repo>`, which gh reads before
the working directory and the `GH_*` variables.

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
log. A deny then records `gate_blocked` with only what is left. SQLite's lock wait applies per
statement, not per record, and opening the ledger and appending runs up to 16 statements that
can each meet another process's lock, so the record waits at most 100 ms per statement and
1.6 s is held back for all 16 waits; git gets the rest, up to its usual 1 s
(`git_and_every_lock_wait_of_the_record_fit_in_what_is_left_of_the_budget`). With less than
200 ms left for git the record is skipped and the deny's reason ends with `(the block was not
recorded in the ledger: no time left in the hook's budget)`; the deny itself always goes out.
The fixed sum (2 s for Clef, 1 s for git, 1.6 s of lock waits) is 4.6 s, over the 4.5 s
budget, and it leaves out the local reads and writes around the Clef call, which is why git's
share is cut to what the clock says is left rather than assumed.

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

**Shadow first.** D1 reaches Workers AI over HTTPS (`docs/decide.md`), and every answer is
logged, but the verdict stays the denylist's until D1 is configured `enforce` and has 50
answered decisions; unanswered calls do not count toward them.

**Redaction** (`hook::guard::redact`). The command Clef gets (state
`{"tool": "Bash", "input": {"command": ...}}`) and the row keeps (its `input` field) is an
allowlist skeleton of it: programs, flags and separators, with every argument value replaced by
`<redacted>` unless a fixed table names it. It is built from the guard's own lexer
(`shell::parse`) and the denylist's wrapper table (`denylist::layout`, from
`denylist::wrappers::unwrap`), the view the denylist judges, so it never reads bash differently
from the guard, and no text the command's author chose reaches Clef unless an allowlist names
it. There are no leaks by design (the scanner it replaces leaked through `$( )` bodies,
wrappers and ANSI-C escapes in four rounds of review); D1 sees less. Then it is trimmed and cut
to 1,024 characters (marked `…`). A command the lexer cannot read (an unclosed quote or
substitution, nesting past 32, a redirection with no target) is `<unparsed>`, nothing else; the
denylist denies it anyway.

Each simple command the lexer lists is rendered in order. Those of `$( )`, backticks, subshells
and process substitutions come out as commands of their own, before the command that holds
them (`x=$(cat f)` is `cat <redacted>; x=<redacted>`). Commands are joined by `; `, or ` | `
when a command reads the pipe of the one just before it: the lexer keeps no other separator,
so `&&`, `||`, `&` and newlines all show as `; `.

- **Arithmetic**: a command the lexer read inside `$(( ))` or `(( ))` (or inside a `( )`
  group in one, or in `$[ ]`) is one `<redacted>`, and a word it read between an unquoted `$[`
  and its `]` is `<redacted>` whatever its role, an assignment still showing its name: their
  operands and operators only look like programs and arguments
  (`sleep $(( 3600 * 24 ))` is `<redacted>; sleep <redacted>`, `(( n = 1 ))` is
  `<redacted>`, `x=$[ n ]` is `x=<redacted> <redacted> <redacted>`). A known gap: bash runs
  `((cmd) )` and `$((cmd) )`, with a blank between the two `)`, as a subshell and a command
  substitution, but the lexer reads them as arithmetic, so their program is hidden from Clef
  too (signal lost, nothing leaked; the denylist still checks them). A `$( )`
  or backtick inside one is a command and is rendered as one
  (`echo $(( $(rm -rf /) + 1 ))` is `rm -rf /; <redacted>; echo <redacted>`). The lexer lists
  the same commands and words as before and only marks these, so the denylist reads them as
  it always has; a `((` that bash reads as two subshells (`((a) )`) is marked too, and an
  unclosed `$[` (bash refuses it) marks the rest of its list, which only hides more.

- **Words before the program**, each by the role the denylist's walk gives it
  (as `denylist::layout` returns them), never by its text alone: keywords (`if`, `then`, `{`, `!`) as written;
  an assignment the denylist reads as one (before the program, or after a wrapper that runs a command:
  `TOKEN=x deploy`, `sudo K=v run`, `env -i A=b cmd`) as `NAME=<redacted>`, a name over 64
  characters hidden too; a word standing in the place of a program that the walk reads as a
  wrapper (`sudo`, `doas`, `env`, `command`, `nohup`,
  `exec`, `time`, `nice`, `timeout`, `setsid`, `stdbuf`, `ionice`, `chrt`, `taskset`,
  `busybox`, `flock`, `xargs`, `coproc`, shells, `eval`, `su`, `script`, `trap`, `watch`,
  `find`, ...) by the program rule, their options by the option rule and their values and
  operands as arguments (`sudo -u <redacted>`, `timeout <redacted>`, `bash <redacted>`). The
  wrapper list and each wrapper's option arity are the denylist's own, so the program after
  any wrapper is shown as a bare one is, and a value or operand that names a wrapper is hidden
  like any other (`sudo -D /x/nice rm` is `sudo -D <redacted> rm`, `find /a/env -delete` is
  `find <redacted> -delete`).
- **Uncertain wrappers**: the redaction never changes what the denylist denies, so it reads
  wrappers with the denylist's walk exactly as it is, and that walk is looser than some
  wrappers are: an option it does not know is a flag, a union of implementations is read
  (`sudo -c x` takes a value Linux sudo refuses), and `chrt -p` or `taskset -p` runs the words
  after the priority or mask. Where that differs from the wrapper's own reading, the word the
  walk places as the program may be an option's value. So a wrapper's words are trusted only
  when each reads with certainty by the redaction's own tables (`guard::redact::certain`): an
  exact flag, an exact value option followed by its value (or with it attached: `-n1`,
  `--user=root`; a shell takes none attached, so `bash -oerrexit` is uncertain), `--`, then
  the wrapper's fixed operands (`timeout`'s duration, `taskset`'s mask, `flock`'s file) and,
  for `sudo` and `env` only, literal assignments, ending exactly where the walk puts the next
  wrapper, the program or the body.
  A word filling a value or operand slot counts only when it has that slot's strict shape, one
  the wrapper takes whatever the environment (a value it refuses makes it run nothing):
  `timeout`'s duration and `-k` a number with an optional `s`, `m`, `h` or `d`, `-s` one of
  the POSIX signals (`KILL`, `SIGTERM`, `9`; `RTMIN+1` or `65` exist on some systems only),
  `taskset`'s mask hex and not 0 (a CPU list such as `0-3,5` after `-c`), `nice -n` an
  integer, `ionice -c` a class and `-n` digits, `watch -n` and `flock -w` a number, `xargs -n`
  and `-L` a count of at least 1 (`xargs -n 0` exits 1), `xargs -P` and `watch -q` digits,
  `stdbuf`'s sizes digits with a unit of at most `G` (and `L` for `-o` and `-e`, not `-i`),
  `env -u` a name without `=`, every number at most nine digits (`find -maxdepth
  99999999999` is out of range), and any other value a word not empty and not starting with
  `-` (`sudo -u -E psql` and `env -C '' psql` are uncertain); every value and operand must be
  literal, since a glob, variable or substitution may become several words or none
  (`sudo -u $U psql`, `flock /tmp/*.lock psql` and `su $U -c …` are uncertain). Options whose
  values the wrapper may refuse whatever their syntax, or under which it may run nothing, are
  not in the tables: `chrt` (its priority's range depends on the policy: `chrt -o 5` is
  refused), `flock -E` (0 to 255), `sudo -C` (at least 3), `xargs -d`, `-s` and
  `--process-slot-var`, `xargs -p`, `env -0` (refused with a command), `doas -s` (refused
  with one) and `setsid -c` (it needs a terminal). `env -S`
  (`--split-string`) and `watch -x` (`--exec`) are never trusted: both split their words
  and run them directly, with no shell, so their string is no shell text to render
  (`env -S 'echo a; hunter2'` is `env -S <redacted>`, `watch -x echo 'a; hunter2'` is
  `watch -x <redacted> <redacted>`). A shell's short-option cluster reads letter by letter:
  each letter one of its no-value flags, `c` only last, the body then the next word, as the
  denylist's walk also reads it (`bash -lc 'rm -rf /'` is
  `bash -<redacted> 'rm -rf /'`; `bash -cl …` and `bash -lsc …` are uncertain). `find`'s
  actions count only when every word of `find` before them reads with certainty: leading
  `-H`, `-L`, `-P`, `-D x`, `-O<n>`, paths, then exact operators, tests and actions, each
  test's value of its shape (`-maxdepth` digits, `-mtime -7` a signed number, `-perm` a mode,
  not `+644`, which find refuses, `-name` a word not starting with `-`), and each action
  ending in `;`, or in `+` right after `{}`: `find . -name -exec x \;` holds no action (find
  reads `-exec` as the pattern) and is rendered as arguments. The tables leave out what keeps
  find from reaching an action whatever the files (`-o`, as in `-print -o -exec x`, `-true`,
  `-false`, `-quit`), what find refuses before it runs anything (`-regex`, `-iregex`,
  `-regextype`, `-printf`, `-files0-from`, `-context`, `-type D`, an empty path), and `-ok`
  and `-okdir`, which run only once the user answers yes. From the first word that does not
  (a cluster longer than one letter, an option the tables do not know, a value or operand
  without its slot's shape or not literal, `env -S`, `watch -x`, `taskset` with `-p` or
  `--pid`, any `chrt`, `xargs` with `-I`, `-i` or `--replace` (it puts each input item in
  place of the replace string in every argument, the program and a body included, so
  `xargs -I Z sh -c 'Z'` runs an input word), an assignment
  after a wrapper that takes none, such as `nohup K=v run`, a non-literal assignment after
  `sudo` or `env`, such as `env K=$V x`, since bash field-splits a wrapper's arguments and
  `V="1 echo"` makes `echo` the program, a body followed by a word starting with `-` or not
  literal, such as `su -c 'x y' -c ls`, since `su` and `script` read options anywhere and the
  last `-c` wins (a shell's body is cut the same way, which hides more than needed for
  `bash -c 'x' -- -y`), and a body its wrapper would refuse to run for what follows it:
  anything after `flock … -c`'s body (flock exits 64), more than one file in all for
  `script` (`script -c 'x' a b` exits 1) or any with `-B`, `-I` or `-O`; likewise a
  shell option the shell may refuse, that runs nothing or that changes what the body runs:
  bash's flags, `-o` names and long options (only before its short ones: `bash -l --norc`
  exits 2), no `-n`, no `-O`, no `-r` (`--restricted`: bash refuses a command name holding
  `/`) and no `-k` (`-o keyword`: bash takes every assignment-shaped word out of the
  arguments, so `sudo -u X=1 psql` runs `sudo -u psql`); for
  `sh`, `dash`, `zsh`, `ksh`, `mksh` and `ash` only POSIX's `-a -C -e -f -i -m -u -v -x` and
  `-o` names (`dash -B` and `sh -h` exit 2, zsh's `-b` ends its options, mksh's `-T` takes
  the next word); `su -s`, which runs the body with that program; `flock --command=x` and
  `flock -cx`, which flock runs as a program's name; and an alias name bash refuses
  (`alias 'a b=x'`); and always, however their words read, `trap`'s action (trap sets none
  when it refuses a signal, and which signals it takes differs between bash, dash and
  systems: `trap 'x' NOSUCH`, `trap 'x' 99`, dash's `DEBUG`), `alias`'s value (it runs only
  when a later command uses the name), and the word after `builtin` (it runs only a shell
  builtin), `busybox` (only one of its applets), `function NAME` (`function f x` is a syntax
  error) and `coproc` (in `coproc NAME ( x )` the word is the coprocess's name)), every word
  of that simple command is an
  argument of no known program, hidden unless a program-free table (the sensitive targets,
  one-letter options, long option names) names it, and no body is rendered:
  `sudo -iu postgres psql` is `sudo -<redacted> <redacted> <redacted>`,
  `xargs -eE psql hunter2` is `xargs -<redacted> <redacted> <redacted>`,
  `watch -dn psql x` is `watch -<redacted> <redacted> <redacted>`, and
  `chrt -p 10 psql 4242` is `chrt -p <redacted> <redacted> <redacted>`,
  `chrt -i mysql -phunter2` is `chrt -i <redacted> -<redacted>`, and
  `trap 'rm -rf /' EXIT` is `trap <redacted> <redacted>`. Clef loses the program
  there; nothing leaks. Signal lost by design: a `timeout` duration in another `strtod` form
  (`1e3`, `0x10`), a `taskset` mask list (`ff,ff`), `bash -eo pipefail -c …` (a cluster
  ending in a value letter), a `find` primary outside the tables (`-fprintf`, `-newerXY`), an
  unquoted glob or variable as a wrapper's or `find`'s value or assignment before what it
  runs (it may expand to several words), a body followed by an option word or an expansion
  (`bash -c '…' -- -x`, `bash -c '…' "$0"`), what `env -S` and `watch -x` run, and (from
  review round 10) whatever follows `chrt`, `builtin`, `busybox`, `coproc` or
  `xargs -I`, and every `trap` action and `alias` value.
- **The program**: its name without a path or `.exe` (`/usr/bin/python3.12` is `python3.12`,
  `./deploy.sh` is `deploy.sh`), kept when the word is literal (no expansion, glob or brace)
  and the name 1 to 64 of `[A-Za-z0-9._+-]`; otherwise `<redacted>` (`"$CMD"`, `./my tool`).
  A wrapper is shown the same way (`/usr/bin/sudo` is `sudo`).
- **Options**: `--` and `-` kept. A long option keeps its name when it is 1 to 64 of
  `[A-Za-z0-9._-]`, its value hidden (`--token=<redacted>`, `--force`); a word that only looks
  like one (`--a:b`, `--$x`) is `<redacted>`. A short option can carry its value attached
  (`mysql -psecret`), so it is kept only when it cannot: one ASCII letter or digit (`-p`,
  `-f`), a cluster of flags the program's table says take no value (`rm -rf`,
  `git clean -fdx`, `docker system prune -af`; tables for `rm`, `git`, `docker`, `podman`,
  `kubectl`, `cp`, `mv`, `chmod`, `chown`, `chgrp`, `ln`, `shred`), or one of the program's
  single-dash flags (`terraform -auto-approve`, `find -delete`, `-exec`, `-name`); any other
  is `-<redacted>`. After a literal `--`, every word up to the next wrapper is an operand and
  shown only as an argument is (`grep -- --token-name f` is `grep -- <redacted> <redacted>`).
- **Arguments**: `<redacted>`, except
  - a subcommand that matters to "destructive or irreversible", for its program only: `git`
    (`push`, `reset`, `clean`, `checkout`, `switch`, `restore`, `branch`, `rebase`,
    `filter-branch`, `filter-repo`, `stash`, `rm`, `drop`, `clear`, `tag`, `update-ref`,
    `reflog`, `expire`, `gc`, `prune`, `worktree`, `remove`, `commit`, `merge`, and `--all`
    for `filter-branch -- --all`), `gh` (`pr`,
    `merge`, `close`, `repo`, `delete`, `archive`, `rename`, `release`, `issue`, `secret`,
    `variable`, `run`, `cancel`, `workflow`, `disable`), `cargo`, `npm`/`pnpm`/`yarn`,
    `docker`/`podman`, `kubectl` (verbs and resource kinds: `delete`, `ns`, `pod`, ...),
    `terraform`/`tofu`, `helm`, `aws`, `gcloud`, `az`, `systemctl`, and `find`'s `;` and `+`;
  - the key of a keyed operand, with its value kept only when it is a sensitive target:
    `dd if=/dev/zero of=/dev/sda bs=1M` is `dd if=/dev/zero of=/dev/sda <redacted>`;
  - a sensitive target, wherever it stands: `/`, `/*`, `~`, `~/`, `~/*`, `.`, `..`, `*`,
    `./*`, `{}`, `HEAD`, `main`, `master`, `origin`, `origin/main`, `origin/master`,
    `/dev/null`, `/dev/zero`, `/dev/random`, `/dev/urandom`, `/dev/stdin`, and block devices
    by an exact pattern (`/dev/sd[a-z]{1,2}[0-9]{0,3}`, likewise `hd`, `vd`, `xvd`;
    `nvme<n>n<n>[p<n>]`, `mmcblk<n>[p<n>]`, `disk<n>[s<n>]`, `rdisk<n>[s<n>]`, `md<n>`).
- **Redirections**, after the words: `> <target>` for each file written (`>`, `>>`, `>|`, `&>`
  and `>&file` all show as `>`), `< <redacted>` for a file read, `<<< <redacted>` for a heredoc
  or here-string; a target is kept only when it is a sensitive target (`> /dev/null`,
  `> /dev/sda`). Comments are dropped.
- **Bodies**: the shell text a wrapper runs (`bash -c`, `sh -c`, `eval`, `su -c`,
  `script -c`, `flock -c`, `watch` without `-x`; the denylist's `Script`, less `env -S`,
  which runs no shell, and less `trap`'s action and `alias`'s value, which may never run and
  are always hidden), when the words it comes from are literal and the words after
  them are what its wrapper lets follow (none starting with `-` or that may expand), is lexed
  and rendered the same way one level deeper, in
  `'…'`, in place of those words (`bash -c 'rm -rf /; git push -f origin main'`,
  `eval 'rm -rf /'`), or after them when it is part of a word (`su --command='rm -rf /'` is
  `su --command=<redacted> 'rm -rf /'`); a body from a word with an expansion is `<redacted>`
  (`bash -c "$CMD"`), one followed by another option word is `<redacted>` with every word
  after it (`su -c 'x y' -c ls` is `su -c <redacted> -c <redacted>`), one that does not lex
  `'<unparsed>'`, and one past 32 levels or the denylist's 4 MiB lexing budget `<redacted>`.
  `find`'s actions are rendered as commands (`find / -exec rm -rf {} ;`), and its own words by
  its tables wherever the walk finds it (`sudo find . -exec rm {} ; -delete`).

**What D1 no longer sees.** Every argument value outside the tables: file names and paths
(`rm -rf build` is `rm -rf <redacted>`), branch names other than `main` and `master`, resource
names (`kubectl delete ns <redacted>`), numbers (`gh pr merge <redacted>`), URLs, messages,
`echo` and `printf` text, option values (`--repo <redacted>`), clusters that may hold a value
(`ls -<redacted>`), the names of declared variables (`export <redacted>`), what a heredoc,
here-string or file on stdin holds, and which of `;`, `&&`, `||`, `&` or a newline separates
two commands. Variables set in the command are not followed (the denylist puts their values in;
the skeleton never shows a value). A program name is shown whatever it is (its path is not), so a secret
written as a program name (`./deploy-hunter2.sh` is `deploy-hunter2.sh`) or as a short assignment name would reach Clef: that
is the design's one accepted exposure. A word the lexer read as part of an arithmetic
expression is never shown as a program, even where the denylist's walk would place one, nor
is a word after a wrapper option the redaction cannot read with certainty. The prompt hooks' prose redaction
(`decide::redact::safe_prompt`, `docs/hooks.md`) is a different, word-rule filter and is
unchanged.

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
