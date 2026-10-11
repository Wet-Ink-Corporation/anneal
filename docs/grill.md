# `anneal grill`: the design tree, its frontier and its confirmation

The `grill` skill questions the human about a new or vague request until they confirm the
requirements. It lands with the `spec` and `plan` skills in #20, because each names the others
in its "Do NOT use for" clause and routing fixtures; this page and `anneal grill` come first. Its state is a **design tree** file the model keeps;
`anneal grill` reads that file deterministically, so the frontier is computed rather than
remembered, and the confirmation is a ledger event rather than a claim.

```text
anneal grill frontier --tree <file>
anneal grill confirm  --tree <file> --quote <the human's words> [--session <id>] [--store <file>]
```

## The design tree

A JSON file, by convention `.anneal/grill/<topic>.json`, valid against
`plugin/playbooks/schemas/design-tree.schema.json` (a test keeps the schema and the Rust DTO in
`crates/anneal-cli/src/grill/tree.rs` in agreement).

```json
{
  "version": 1,
  "goal": "Export orders as CSV for the finance import job",
  "turns": [{ "confidence": 0.4 }, { "confidence": 0.65 }],
  "nodes": [
    {
      "id": "scope",
      "question": "Which orders are exported?",
      "depends_on": [],
      "status": "answered",
      "answer": "Shipped orders of the previous day",
      "is_decision": 0.9,
      "confidence": 0.9
    },
    {
      "id": "format",
      "question": "Which columns, in which order?",
      "depends_on": ["scope"],
      "status": "open",
      "answer": "The columns of the finance import template, in its order",
      "is_decision": 0.8,
      "confidence": 0.6
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `version` | Format version: `1`. |
| `goal` | What is being grilled, in one line. Not blank. |
| `turns` | One `{"confidence": c}` per round, oldest first; `c` from 0 to 1 is the model's confidence in the shared understanding after the round. At most 256. |
| `nodes` | The questions. At most 512. |
| `nodes[].id` | Unique; `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. |
| `nodes[].question` | The question as it would be asked. Not blank. |
| `nodes[].depends_on` | Ids of the questions whose answers this one needs first. Each names a node, at most once; no cycles. |
| `nodes[].status` | `open`, `answered` or `deferred`. |
| `nodes[].answer` | While open, the **recommended answer** a plain "yes" accepts; once answered, the agreed answer; while deferred, the assumption carried (for an out-of-scope ruling, its id). Never blank: every question carries a recommendation. |
| `nodes[].is_decision` | Probability from 0 to 1 that only the human can settle it. From 0.5 it is asked; below, the model looks it up. Clef D7 (#32) will write its estimate here. |
| `nodes[].confidence` | The model's confidence in `answer`, from 0 to 1. |

Unknown fields are rejected. The file is read up to 1 MiB.

## `anneal grill frontier`

Validates the tree and lists the **frontier**: the `open` nodes whose `depends_on` are all
`answered`, sorted by id. A node waiting on an open or a deferred node is not on it.

The envelope's `data`:

| Key | Meaning |
|---|---|
| `tree`, `hash`, `goal` | The file as given, `sha256:` and the 64 hex digits of its bytes, its goal. |
| `nodes`, `open`, `answered`, `deferred` | Counts. |
| `frontier` | The frontier nodes, as written in the file. |
| `ask`, `look_up` | The frontier's ids split by `is_decision` (from 0.5: ask the human; below: look it up). |
| `confidence` | `turns`, `latest`, `best`, `turns_without_gain`, `restate`, and the thresholds `restate_at` (0.8) and `restate_after_turns_without_gain` (3). |

`turns_without_gain` counts the latest turns in a row whose confidence did not beat the best
confidence before them. `restate` is the restate gate: the latest confidence is 0.8 or more, or
3 turns in a row gained nothing. The skill stops asking and restates the design when it is true.

## `anneal grill confirm`

Records the human's explicit confirmation of the tree as one `grill_confirmed` ledger event
(the registry's existing type; there is no dotted `grill.confirmed` name), in this order:

1. `--quote` must hold the human's words, verbatim and not blank. Missing or blank is a usage
   error (exit 2, one `ERROR` line, nothing on stdout): confirmation is never inferred.
2. The tree must be valid (else exit 1 with its findings).
3. No node may be `open`: answer it or defer it first (else exit 3, one `grill/open:<id>`
   finding per open node).
4. The event is appended to the ledger (resolved as for `anneal ledger`; `--store` overrides):
   `{"type": "grill_confirmed", "plan": "<the --tree path as given>", "hash": "sha256:<hex of
   the bytes validated>", "quote": "<the words>"}`, actor `cli`, with `session_id` from
   `--session`.

Nothing is appended in steps 1 to 3. The hash is of exactly the bytes that were validated, so a
later check can tell whether the confirmed tree has changed since. Later gates (#23) read the
event; until then nothing refuses edits without it.

What the command cannot check is whether the quoted words are the human's. The skill's MUST
says to run it only after the human's explicit yes to the restatement, quoting them; the quote is
kept in the ledger so a reviewer can read it.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | ok: the frontier was listed, or the confirmation recorded |
| 1 | invalid design tree: a cycle, a dangling or repeated dependency, a duplicate or bad id, a blank goal, question or answer, a probability outside 0 to 1, a bound exceeded, or JSON that is not a design tree |
| 3 | `confirm`: a question is still open |

A missing `--tree` file is a usage error (2); the global codes are in
[exit-codes.md](exit-codes.md).

## Finding keys

`grill/json`, `grill/too-large`, `grill/version`, `grill/empty-goal`, `grill/too-many-turns`,
`grill/too-many-nodes`, `grill/confidence:turn-<n>`, `grill/bad-id:<id>`,
`grill/duplicate-id:<id>`, `grill/empty-question:<id>`, `grill/empty-answer:<id>`,
`grill/confidence:<id>`, `grill/is-decision:<id>`, `grill/dangling:<id>:<dependency>`,
`grill/duplicate-dependency:<id>:<dependency>`, `grill/cycle:<a>-<b>-...-<a>` (one cycle, from
its smallest id) and, for `confirm`, `grill/open:<id>`. A cycle is looked for only once the ids
are unique and every dependency names a node.
