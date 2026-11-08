---
name: grill
description: Use when asked for new behaviour without saying how it works ("let users X", "make every Y do Z"), even if it sounds precise, or a request or idea is vague; question the human in rounds, each with a recommended answer, until confirmed. Do NOT use for the spec (use `spec`) or a bug (use `diagnose`).
---

# Grill

## Overview

Turns a new or vague request into requirements the human confirmed. The questions live in a design tree file (`${CLAUDE_PLUGIN_ROOT}/playbooks/schemas/design-tree.schema.json`).

- You MUST keep every question in the design tree and run `anneal grill frontier --tree <file>` before each round; ask from its `ask` list, nothing else.
- You MUST give every question a recommended answer that a plain "yes" accepts; `anneal grill frontier` rejects a node with a blank `answer`.
- You MUST run `anneal grill classify --tree <file> --node <id>` on each new open node, and look up, not ask, every `look_up` node of `anneal grill frontier` (`is_decision` below 0.5).
- You MUST run `anneal grill confirm --tree <file> --quote "<their words>"` only after the human explicitly confirms your restatement, quoting them verbatim; it records `grill_confirmed`.
- Without Bash or Write, ask each round inline, numbered, with recommendations.

## When to use

- A request without agreed requirements: a feature idea, "make X better", an unstated scope.
- Not when requirements are agreed (use `spec`), nor when a spec exists (use `plan`).

## Process

1. **Seed the tree.** Write `.anneal/grill/<topic>.json`: `version: 1`, a one-line `goal`, empty `turns`, and the root questions (goal, users, scope, constraints, done-when) with a recommended `answer`, `is_decision`, `confidence` and `depends_on` (only questions that change its answer). It's working if `anneal grill frontier` exits 0.
2. **Look facts up.** Answer each `look_up` id from the code, docs or tracker, or a subagent given the paths; mark it `answered`. It's working if no fact reaches the human.
3. **Builder check.** Ask only the `ask` ids where a wrong guess changes the build; accept your recommendation for the rest, in one line the human can object to. It's working if every question would change the build.
4. **Ask one round.** Number the questions, each recommending what serves the goal, and why; "yes" accepts all. Record answers and statuses, and append the round's confidence to `turns`. Label asked nodes (classify's `data.id`): `anneal decide label <id> no` if the human says you could have looked it up, else `yes`. It's working if the next frontier drops the answered questions.
5. **Restate at the gate.** When `data.confidence.restate` is true, stop asking: restate the goal, decisions and deferrals, and ask if it is right. It's working if the human answers it.
6. **Confirm.** Answer or defer every open node, then run `anneal grill confirm` with the human's exact words. It's working if its event carries the tree's hash.
7. **Keep language and scope.** Corrected terms go in references/domain-language.md, out-of-scope rulings in references/scope-ledger.md. It's working if each has its entry.

## Rationalizations

| Excuse | Reality |
|---|---|
| "The request is clear enough" | Clear to you. The tree shows the open branches. |
| "I'll ask it as an open question" | Without a recommendation the human does your design work. |
| "They said ok, that counts" | Only an explicit yes to the restatement counts, quoted verbatim. |
| "Faster to ask which library we use" | That is a fact. Look it up. |
| "We ship in minutes, just build it" | A wrong guess costs more than one round of questions. |

## Red flags

- Asking what the code, docs or tracker answer.
- Running `anneal grill confirm` with words the human did not write, or before the restatement.
- Editing source before `grill_confirmed` is in the ledger.

## Verification

- `anneal grill frontier --tree <file>` exits 0 with no open node.
- `anneal ledger query --type grill_confirmed` shows the tree's path, its `sha256:` hash and the human's words.
