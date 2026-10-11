# Specs: the template and `anneal lint spec`

A spec is written from `plugin/playbooks/spec.template.md`, in the same session as the
interview that confirmed its requirements, and only from what was confirmed. `anneal lint spec`
checks what a script can check: the sections, the shape of every acceptance criterion, the
open questions and the constitution check.

```text
anneal lint spec <FILE> [--glossary CONTEXT.md] [--constitution constitution.md]
```

## Rules

Every finding is an error with a fix string. The run exits 0 when there is none and 1 when
there is any (`lint`'s codes, `docs/exit-codes.md`).

| Rule | What is checked | Where the finding points |
|---|---|---|
| `sections` | The level-2 sections `Problem`, `Users`, `Requirements`, `Acceptance criteria`, `Out of Scope`, `Open questions` and `Constitution check` are present (titles in any letter case, in any order), each once | line 0 for a missing one; the repeat's heading for a second one |
| `ears` | `Acceptance criteria` has at least one list item, and every list item parses as EARS (below) | the item; the heading when there is none |
| `out-of-scope` | `Out of Scope` has visible text (an HTML comment alone is empty) | the heading |
| `needs-clarification` | At most 3 `[NEEDS CLARIFICATION]` markers | the marker that makes four |
| `constitution` | The constitution check against the constitution (below) | the line at fault, in the spec or in the constitution |
| `avoided-term` | No term the glossary lists on an `_Avoid_:` line, matched as `lint terms` matches (`docs/define.md`) | the line, one finding per term |

## Inputs

- `<FILE>` is the spec. A missing file, a directory or a file over 1 MiB is a usage error
  (exit 2, one `ERROR` line, nothing on stdout). Text that is not UTF-8 is read lossily.
- `--glossary` defaults to `CONTEXT.md` and `--constitution` to `constitution.md`, both in
  the working directory. A default that does not exist is skipped, and `data` shows `null`
  for it. A file named with the flag must exist: a missing one is a usage error.

## Output

The JSON envelope, `command: "lint spec"`, with `data`:

```json
{
  "file": "docs/specs/export.md",
  "glossary": "CONTEXT.md",
  "constitution": {"file": "constitution.md", "version": "3", "gates": 3},
  "criteria": 5,
  "needs_clarification": 1,
  "findings": [
    {"key": "lint/spec/ears:docs/specs/export.md:17", "rule": "ears",
     "file": "docs/specs/export.md", "line": 17,
     "message": "acceptance criterion is not EARS: has no `SHALL`.",
     "fix": "write it as `WHEN <trigger>, THE <system> SHALL <response>.`"}
  ]
}
```

- `needs_clarification` is always present: the number of markers, however many. `plan
  approve` (#20) blocks while it is above 0.
- `glossary` and `constitution` are `null` when none was read; `constitution.version` is
  `null` when the constitution has no version line.
- Each finding has `rule`, `file`, `line` (0 for the file as a whole), `message`, `fix`, and a
  `subject` (the section, gate or term) when one line can hold several findings. Its `key`,
  `lint/spec/<rule>:<file>:<line>[:<subject>]`, is stable across runs. Findings are sorted by
  file, line, rule and subject, and each is also an envelope finding whose message is one line,
  `<file>:<line>: error [<rule>] <message> fix: <fix>`.

## What the lint reads

The markdown is read by hand:

- **Hidden text.** Fenced code blocks (```` ``` ```` or `~~~`, closed by a fence of the same
  character at least as long) and HTML comments (`<!-- -->`, also across lines) are not read.
  Headings, list items, markers and gates inside them do not count. Avoided terms are the
  exception: like `lint terms`, they are looked for in the whole file.
- **Sections.** A section is a `## ` heading (closing `#`s allowed) and the lines up to the
  next `# ` or `## ` heading. Deeper headings (`### Complexity`) belong to their section.
- **List items.** A list item is a line starting with `-`, `*` or `+`, or digits followed by
  `.` or `)`, then a space or the line end, at any indentation. Indented lines after it join
  the item with a space. A blank line or any other line ends it. Nested items are items of
  their own.
- **Markers.** A marker is `[NEEDS CLARIFICATION]` or `[NEEDS CLARIFICATION: <question>]`,
  in capitals, outside inline code (backticks).

## EARS grammar

Each list item under `Acceptance criteria` is one criterion:

```text
criterion := [label] [WHERE text] [WHILE text] [WHEN text | IF text THEN] THE system SHALL response
```

| Form | Template |
|---|---|
| ubiquitous | `THE <system> SHALL <response>.` |
| event-driven | `WHEN <trigger>, THE <system> SHALL <response>.` |
| state-driven | `WHILE <state>, THE <system> SHALL <response>.` |
| unwanted behaviour | `IF <condition>, THEN THE <system> SHALL <response>.` |
| optional feature | `WHERE <feature>, THE <system> SHALL <response>.` |
| complex | the clauses combined, such as `WHERE <feature>, WHILE <state>, WHEN <trigger>, THE <system> SHALL <response>.` |

- **Keywords** are `WHERE`, `WHILE`, `WHEN`, `IF`, `THEN`, `THE` and `SHALL`, as whole words
  in capitals. A trailing `,`, `.`, `:` or `;` is ignored, and commas between clauses are
  optional. A word in lowercase (`when`, `the`) is ordinary text.
- **Clauses** come in EARS order: `WHERE`, then `WHILE`, then `WHEN` or `IF` (not both),
  each at most once. A clause's text runs up to the next keyword, so the keywords are reserved
  inside clauses: write `WHEN the user saves`, not `WHEN THE USER saves`.
- **`IF`** needs `THEN` after its condition. `THEN` without `IF` is an error.
- **The main clause** is `THE`, the system (at least one word), `SHALL` and the response (at
  least one word). Anything may follow `SHALL`.
- **A label** may come first and is skipped: a checkbox (`[ ]`, `[x]`), a bold label followed
  by a colon (`**AC-1**:`), or a first word that holds a digit and ends with a colon (`AC-1:`).

A criterion that does not parse is an `ears` finding naming the first problem met, left to
right: an empty item, a first word that is not a keyword, a clause out of order or repeated,
both `WHEN` and `IF`, a clause with no text, `IF` without `THEN`, `THEN` without `IF`, `SHALL`
with no `THE <system>` before it, `THE` with no system, no `SHALL`, or no response. Its fix is
the closest template: the clauses whose keywords, in any letter case, start the item or follow
a comma, in EARS order, then `THE <system> SHALL <response>.` So `When the user saves, the
editor stores it` gets `WHEN <trigger>, THE <system> SHALL <response>.`

## Constitution check

A project's constitution (`constitution.md`) holds the gates every spec is checked against.
It is versioned: raise the version whenever a gate changes, so specs checked against an older
version are found.

```markdown
# Constitution

**Version**: 3 | **Ratified**: 2026-10-11

## Gates

- [G1] Library first: every feature starts as a library.
- [G2] One store: state lives in the ledger, nowhere else.
- [G3] Test first: no code without a failing test.
```

- **Version line.** The first line reading `Version: <v>` or `Constitution version: <v>`, in
  any letter case, after an optional list marker, with `*` and `_` emphasis ignored. `<v>` is
  the first word after the colon, up to any `|`, compared as text (`3`, `1.2.0`).
- **Gates.** List items that open with an id in brackets. An id is two or more ASCII letters,
  digits, `-`, `_` or `.`, starting with a letter (`G1`, `I-2`), so a `[x]` checkbox is not a
  gate. Ids are stable: never reuse one for a different gate.

The spec's `## Constitution check` names the version and gives every gate a status, `pass`,
`violated` (with the reason after it) or `n/a`. A violated gate needs a row in a complexity
table in the same section that says what breaks the gate and why the simpler option is not
enough:

```markdown
## Constitution check

Constitution version: 3

- [G1] pass
- [G2] violated: the offline queue is kept on disk beside the ledger.
- [G3] n/a

### Complexity

| Gate | Violation | Justification |
|---|---|---|
| G2 | a disk queue beside the ledger | the ledger is unreachable while offline |
```

`constitution` findings:

| Finding | Where |
|---|---|
| The constitution has no version line | the constitution, line 0 |
| A gate id appears twice in the constitution | the constitution, the repeat |
| The section names no version, or a version other than the constitution's | the section heading, or the version line |
| A constitution gate the section does not list | the section heading |
| A gate entry repeated, naming a gate the constitution lacks, or with no `pass`, `violated` or `n/a` (one finding per entry, in that order) | the entry |
| A violated gate with no row in the `\| Gate \| Violation \| Justification \|` table (header in any letter case; the gate cell may hold `G2`, `[G2]` or `` `G2` ``) | the entry |
| A row with an empty violation or justification, a row naming no gate, or a row for a gate not marked violated | the row |

Without a constitution, the version and the gate list are not checked, but every gate the spec
marks violated still needs its justified row. Without the section, only the constitution's own
findings are reported (the missing section is a `sections` finding).
