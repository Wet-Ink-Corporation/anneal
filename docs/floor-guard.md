# `anneal floor-guard`

`anneal floor-guard [--base <ref>]` refuses a change that lowers a quality floor. It reads the
diff of the working tree (committed and uncommitted changes) against the merge base of `HEAD`
and the base, plus every untracked file git does not ignore (read as all added), and reports
one finding per hit.

| Exit | Meaning |
|---|---|
| 0 | no floor lowered |
| 1 | a floor was lowered: `data.findings` lists each hit |
| 2 | tool error: one `ERROR floor-guard: ...` line on stderr and nothing on stdout |

Exit 2 covers everything that stops the check from reading the whole diff: not in a git
repository, a `--base` that does not resolve to a commit, no `--base` while neither
`origin/main` nor `main` resolves, `HEAD` and the base with no merge base, a git failure or a
git call past its 60 s deadline, and `git diff` or `git ls-files` output over 16 MiB. A check
that did not run never reads as 0.

The base defaults to `origin/main` when it resolves, else `main`.

## What it detects

Every detection is diff-scoped: only what the change adds or removes counts, never what was
already there.

| Category | Hit |
|---|---|
| `suppression` | An added line holds `#[allow(` or `#![allow(`, `#[ignore]` (or `#[ignore = ...]`), `// eslint-disable` or `/* eslint-disable` (any variant), `# noqa` (any case), `.skip(` on `it`, `test`, `describe`, `context`, `suite`, `pytest`, `unittest` or `mark` (so an iterator's `.skip(1)` is not one), or `xit(` as a whole word (so `exit(` is not one). Prose files (`.md`, `.markdown`, `.rst`, `.txt`, `.adoc`) are skipped. `#[expect(` is not flagged: temper judges it. |
| `deleted_test` | A test file was deleted. A test file is one under a `tests/` directory, or named `*_test.*`, `*.test.*`, `*.spec.*` or `test_*.py`. A rename (git's `-M`) to another test path is not a deletion; a rename out of the test paths (`tests/test_x.py` to `old/x_py.bak`) is, reported at the old path. |
| `emptied_test` | A test file that had lines removed is blank (empty or whitespace) in the working tree. |
| `removed_test` | Across the whole diff, more test declarations were removed than added; each file that removed more than it added is reported at its first removed declaration (a line in the base). A test moved from one file to another nets to zero. Declarations: `#[test]` and `#[tokio::test` in `.rs`, `it(` and `test(` at the start of a line in JavaScript and TypeScript, `def test_` and `async def test_` in `.py`. |
| `coverage_lowered` | A coverage floor got a lower number: `fail_under`, `fail-under`, `--fail-under`, `min_coverage`, any key ending `-fail-under` or `_fail_under` (`--cov-fail-under`), and `lines`, `branches`, `functions` and `statements` in a file whose working-tree content mentions `coverageThreshold`. |
| `timeout_raised` | A timeout got a higher number: `timeout`, `timeout_ms`, `testTimeout`, `timeout-minutes`. |
| `ratchet_loosened` | In `.anneal/ratchets.json` (see Ratchets below), a ratchet moved the wrong way since the merge base, was removed or changed direction, or the file no longer parses. In any other `*ratchet*.json` file (another tool's), a key starting `max` got a higher number or one starting `min` a lower one. |

A threshold is a key followed by `=`, `:` (or a space after a `--flag`) and a number, with
optional quotes, as in `fail_under = 80`, `"testTimeout": 5000`, `timeout-minutes: 30` and
`--fail-under 80`. An added threshold is compared with the first removed one of the same key
in the same hunk; a threshold with no removed counterpart (a new one) is not judged.

Each finding in `data.findings` is `{key, category, file, line, message}`; `line` is `null`
for a whole file (`deleted_test`, `emptied_test`), and the `key`
(`floor/<category>:<file>[:<line>]`) is stable across runs. The envelope's `findings` carry
the same keys and messages. At most 1,000 findings are listed, sorted by file, line and
category; the rest are counted in `data.omitted`. The cap holds while findings are collected, so memory stays bounded however large the diff. An untracked file over 1 MiB is not scanned,
and a changed file over 1 MiB gets no emptied-test or `coverageThreshold` check; both are
listed in `data.skipped` with the reason, never dropped silently. `data` also holds `base`,
`base_commit`, `merge_base`, `files` (changed files in the diff) and `untracked`.

## Ratchets

`.anneal/ratchets.json` holds numeric floors, such as a warnings count or coverage, that may only
move the good way:

```json
{"version": 1, "ratchets": {
  "warnings": {"value": 12, "direction": "down"},
  "coverage": {"value": 81.5, "direction": "up"}}}
```

- `direction` is `down` (lower is better) or `up` (higher is better); `value` is a JSON number.
- Names are 1 to 64 of `A-Z a-z 0-9 _ . -`; at most 256 ratchets; no other field is allowed and
  `version` must be 1.
- floor-guard does not compute the metrics. It guards the committed numbers: when the diff
  touches the file, it reads the merge base's copy (`git rev-parse <merge-base>:.anneal/ratchets.json`,
  then `git cat-file blob`, at most 1 MiB) and the working tree's.
- Each base ratchet that rose (`down`) or fell (`up`), was removed, or changed direction is a
  `ratchet_loosened` finding at the line that names it (no line when removed). A deleted file
  removes every ratchet. A working-tree file that does not parse, or is over 1 MiB, is one
  finding for the whole file: breaking the file never unblocks a floor.
- A ratchet new since the base, or a file new since the base, is not judged. A base copy that is
  not valid (or is over 1 MiB) leaves nothing to compare and is listed in `data.skipped`.
- The generic `*ratchet*.json` rule above skips `.anneal/ratchets.json`, so one change is never
  reported twice; it stays for other tools' ratchet files.

The `fast-check` hook runs floor-guard after every batch of edits and blocks on a lowered floor
(`docs/hooks.md`).

## Limits

- It reads text, not syntax: a pattern inside a string literal or a comment counts, so code
  that names the patterns (a linter, or this command's own tests) is reported.
- Paths are read with `core.quotePath=false`, and the TAB git appends to a `---`/`+++` path with a space is dropped; the diff is `-U0` with `a/` and `b/` prefixes,
  no external diff and no text conversion, so user diff settings do not change what it reads.

## Floor guard and temper

temper's escape-hatch scan and floor guard run side by side; neither replaces the other.

- **temper** checks the whole Rust tree at every gate for escape hatches: `unsafe`, `unwrap`
  and `expect`, `panic!` and friends, panicking indexing, `#[allow]`, unapproved `#[expect]`,
  lossy casts, debug leftovers, and weakened lint, CI or hook config. It is Rust-only and
  judges the code as it stands.
- **floor guard** is diff-scoped and language-agnostic. Beyond suppressions it covers what a
  code scan cannot see: deleted, emptied and removed tests, and loosened coverage floors,
  timeouts and ratchet values, in any language and in configuration files.

Floor guard does not repeat temper's Rust checks. The one overlap is deliberate: `#[allow(` and
`#[ignore]` are in the story's list, so floor guard reports them on added lines too. That is
cheap, and covers repositories that do not run temper. `#[expect(` stays temper's alone.
