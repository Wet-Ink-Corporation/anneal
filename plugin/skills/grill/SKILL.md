---
name: grill
description: Use when a request or idea is new or vague, even if it looks clear; question the human in rounds, each with a recommended answer, until they confirm the requirements. Do NOT use for writing the spec once requirements are agreed (use `spec`) or splitting agreed work into tasks (use `plan`).
---

# Grill

## Overview

Turns a new or vague request into requirements the human confirmed, before anything is specified or built. The questions live in a design tree file (`${CLAUDE_PLUGIN_ROOT}/playbooks/schemas/design-tree.schema.json`).

- You MUST keep every question in the design tree and run `anneal grill frontier --tree <file>` before each round; ask from its `ask` list, nothing else.
- You MUST give every question a recommended answer that a plain "yes" accepts; `anneal grill frontier` rejects a node with a blank `answer`.
- You MUST look up, not ask, every node `anneal grill frontier` lists under `look_up` (`is_decision` below 0.5; Clef D7, #32, will score it).
- You MUST run `anneal grill confirm --tree <file> --quote "<their words>"` only after the human explicitly confirms your restatement, quoting them verbatim; it records `grill_confirmed`.

## When to use

- A request arrives without agreed requirements: a feature idea, "make X better", a change with unstated scope.
- Not when requirements are agreed (use `spec`), nor when a spec exists (use `plan`).

## Process

1. **Seed the tree.** Write `.anneal/grill/<topic>.json`: `version: 1`, a one-line `goal`, empty `turns`, and the root questions (goal, users, scope, constraints, done-when) with `depends_on`, a recommended `answer`, `is_decision` and `confidence`. It's working if `anneal grill frontier --tree <file>` exits 0.
2. **Look facts up.** Answer each `look_up` id from the code, docs or tracker (or hand a subagent the file paths and the question); mark it `answered`. It's working if no fact reaches the human.
3. **Builder check.** Of the `ask` ids, keep those where a wrong guess changes what gets built; accept your recommendation for the rest, listed in one line the human can object to. It's working if every question asked would change the build.
4. **Ask one round.** Number the questions, each with your recommendation and why; "yes" accepts all. Record answers and statuses, and append the round's confidence to `turns`. It's working if the next frontier drops the questions just answered.
5. **Restate at the gate.** When `data.confidence.restate` is true (confidence 0.8 or more, or 3 turns without a gain), stop asking: restate the goal, every decision and every deferral, and ask whether it is right. It's working if the human answers the restatement.
6. **Confirm.** Answer or defer every open node, then run `anneal grill confirm` with the human's exact words. It's working if it exits 0 and its event carries the tree's hash.
7. **Keep language and scope.** Corrected terms go into the glossary (references/domain-language.md); out-of-scope rulings into the ruling store (references/scope-ledger.md). It's working if each has its entry.

## Rationalizations

| Excuse | Reality |
|---|---|
| "The request is clear enough" | Clear to you. The tree shows the unanswered branches. |
| "I'll ask it as an open question" | Without a recommendation the human does your design work. |
| "They said ok, that counts" | Only an explicit yes to the restatement counts, quoted verbatim. |
| "Faster to ask which library we use" | That is a fact. Look it up. |
| "One more round will lift confidence" | Three flat turns mean restate now. |

## Red flags

- Asking what the repository, docs or tracker already answer.
- A question with no recommended answer.
- Running `anneal grill confirm` with words the human did not write, or before the restatement.
- Editing source before `grill_confirmed` is in the ledger.

## Verification

- `anneal grill frontier --tree <file>` exits 0 with no open node.
- `anneal ledger query --type grill_confirmed` shows the tree's path, its `sha256:` hash and the human's words.
