# Define-phase helpers

Three deterministic helpers back the Define phase: a term scan for the glossary
(`domain-language`), ADR numbering, and the out-of-scope ruling store (`scope-ledger`). The
term scan is a command, `anneal lint terms`. ADR numbering and the ruling list are library
functions in the `anneal` CLI crate (`anneal_cli::adr`, `anneal_cli::rulings`) for the hooks
and commands that use them (the fast-check hook, #24; D6, #31). No command exposes them yet.

## Term scan: `anneal lint terms`

```text
anneal lint terms [--glossary CONTEXT.md] <FILE>...
```

The glossary lists the words a project has agreed not to use. Each entry that has them carries
an `_Avoid_:` line:

```markdown
**Order**: what a customer asks for.
_Avoid_: purchase, purchase order, `txn`.
```

- **Entries.** Every line containing `_Avoid_:` is an entry. Whatever comes before the
  marker (a bullet, the defined term) is ignored. The terms are the text after it, split on `,`
  and trimmed of spaces, backticks and a final `.`. A term listed twice, in any letter case,
  counts once, and runs of whitespace inside a term count as one space.
- **Hits.** A term is used when it appears in a line in any letter case (Unicode lowercase),
  with a non-word character or the line edge on each side. Word characters are Unicode letters
  and digits plus `_`, so `txn` is not found in `txns`, `mytxn` or `txn_id`, but it is found in
  `(txn)` and `a-txn-b`. A multi-word term matches across single spaces: `purchase order`
  matches `Purchase ORDER` but not `purchase  order` or a line break.
- **Not hits.** A line containing `_Avoid_:` is a glossary entry and is never scanned, so a
  changed `CONTEXT.md` does not flag itself.
- **Output.** The usual JSON envelope, `command: "lint terms"`, with `data`:
  `{"glossary", "terms", "checked", "skipped": [{"file", "size"}], "hits": [{"file", "line",
  "term"}]}`. There is one hit per file, line and term, sorted by file, line and term, and
  each hit is also an envelope finding keyed `lint/terms:<file>:<line>:<term>`.
- **Exit codes.** These are `lint`'s codes: 0 when nothing is found, 1 on any hit. No file
  at all, a missing glossary or file, a directory, or a glossary over 1 MiB is a usage error
  (2, one `ERROR` line, nothing on stdout).
- **File names.** A file is shown without its `.` components and scanned once however often
  it is named, so `a.md` and `./a.md` are one file, shown as `a.md`. Nothing else is
  normalised: `..` and symlinks are kept as given.
- **Large and binary files.** A file over 1 MiB is not scanned. It is listed under `skipped`
  with its size, so it is never passed over silently. A file that is not UTF-8 is read
  lossily: a replacement character is not a word character, so it never creates a hit.

`lint spec` (#18) can call the same function (`anneal_cli::lint::terms::Glossary::scan`).

## ADR numbering

ADRs live in `docs/adr/` as `NNNN-<slug>.md`: four or more digits, `-`, a slug, `.md`. Other
files there (a `README.md`, a template) are not ADRs.

- **Slug.** The title in lowercase ASCII letters and digits, with every other run of characters
  turned into one `-` and none at either end. `ADR #12: v2 API` becomes `adr-12-v2-api`.
  Non-ASCII letters are separators, so the file name reads the same on every file system. A
  title with no ASCII letter or digit is an error.
- **Next number.** A new title gets the highest number plus one, zero-padded to four digits
  (`0001` for the first). Numbers are never reused, so a gap left by a deleted ADR stays.
- **Idempotent.** A title whose slug matches an existing ADR's slug (compared as slugs, so
  `0003-Use-Postgres.md` matches "use postgres") gets that ADR back, not a new number. Asking
  twice gives the same answer, and nothing is written. An existing ADR keeps its file name as
  found, so `00012-x.md` is never renamed to `0012-x.md`.
- **Order.** ADRs sort by number, then slug. When two branches both take `0005`, both files
  survive the merge and list in the same order everywhere. The next title gets `0006`.

`adr::place(dir, title)` reads only file names. A missing directory has no ADRs yet.

## Ruling store

Out-of-scope rulings live in `.out-of-scope/`, one `<slug>.md` per ruling, with frontmatter in
the same restricted YAML as a `SKILL.md`:

```markdown
---
id: no-gui
ruled_at: 2026-10-11
summary: No GUI in v0.1; the CLI is the surface.
weigh_in_ref: wi-2415fc
---

# No GUI

Why it was ruled out, in prose.
```

| Key | Required | Value |
|---|---|---|
| `id` | yes | One word of printable ASCII, unique in the store. D6 offers it as a `choice` option |
| `ruled_at` | yes | `YYYY-MM-DD`, optionally followed by `T` and a time |
| `summary` | yes | One non-empty line |
| `weigh_in_ref` | no | One word naming the weigh-in that decided it |

No other key is allowed, which catches a misspelled key early.

`rulings::list(dir)` reads every regular `*.md` file in the store (`.MD` too) and returns the rulings
sorted by id in byte order, so the options come out in the same order on every run (stable
content first, for prompt caching). A missing store has no rulings. A ruling that does not
read is an error naming its file, and the list fails rather than skipping it. Causes are a
missing or unknown key, an invalid value, no frontmatter, a file that is not UTF-8 or is over
1 MiB, or an id that another file already uses (the error names both files).
