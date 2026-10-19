---
name: spec
description: Use when requirements are agreed, even if the spec seems obvious; write the spec or its acceptance criteria (EARS, SHALL), Out of Scope and the constitution check of its gates. Do NOT use for a new or vague request (use `grill`) or splitting a spec into tasks (use `plan`).
---

# Spec

## Overview

Turns requirements the human confirmed into a spec a script can check, written in the same session as the interview and only from what it confirmed. The template is `${CLAUDE_PLUGIN_ROOT}/playbooks/spec.template.md`.

- You MUST write the spec from the template and run `anneal lint spec <file>` until it exits 0, applying each finding's `fix`.
- You MUST write every acceptance criterion in EARS (`WHEN <trigger>, THE <system> SHALL <response>.` and its variants); `anneal lint spec` rejects any other form.
- You MUST mark each point the human did not confirm `[NEEDS CLARIFICATION: <question>]` instead of guessing; `anneal lint spec` allows at most 3 and reports the count.
- You MUST check the spec against `constitution.md` and justify every violated gate in the Complexity table; `anneal lint spec` checks the version, the gates and the justifications.

## When to use

- The requirements are confirmed (a `grill_confirmed` event is in the ledger, or the human says they are agreed) and no spec exists yet.
- Not when the request is still new or vague (use `grill`), nor when the spec is written and needs tasks (use `plan`).

## Process

1. **Copy the template.** Copy it to `docs/specs/<topic>.md`. It's working if all seven `##` sections are there.
2. **Fill from what was confirmed.** Problem, Users and Requirements in the human's words, from the design tree and the conversation, nothing else. It's working if every requirement traces to a confirmed answer.
3. **Write the criteria.** One list item per observable behaviour, in the EARS form that fits: `WHEN`, `WHILE`, `IF … THEN`, `WHERE`, or ubiquitous. It's working if each criterion could become one test.
4. **Scope it out.** List what this spec will not do, and why; record each out-of-scope ruling in `.out-of-scope/`. It's working if Out of Scope is not empty.
5. **Mark the gaps.** Give each unconfirmed point a marker and an Open questions entry. It's working if nothing unconfirmed reads as fact.
6. **Check the constitution.** Read `constitution.md`, write its version, mark each gate `pass`, `violated: <why>` or `n/a`, and add a Complexity row per violation. It's working if every gate has a status.
7. **Lint.** Run `anneal lint spec docs/specs/<topic>.md` and apply each fix. It's working if it exits 0.
8. **Close the questions.** Ask the human each open question with a recommended answer, then remove its marker. It's working if `data.needs_clarification` is 0 before `plan`.

## Rationalizations

| Excuse | Reality |
|---|---|
| "I know what they meant" | Then it is easy to confirm. Mark it and ask. |
| "Prose criteria are clearer" | Prose cannot be checked. EARS can become a test. |
| "Nothing is out of scope" | Then scope is unbounded. Name what you will not do. |
| "The gate does not apply here" | Then mark it `n/a` and say why in one line. |
| "The violation is obviously fine" | Write the justification; the reviewer reads the table, not your mind. |

## Red flags

- A requirement nobody confirmed, written as fact.
- A criterion without `SHALL`, or with two behaviours in one item.
- Starting `plan` while `data.needs_clarification` is above 0.
- Editing `constitution.md` to make a gate pass.

## Verification

- `anneal lint spec docs/specs/<topic>.md` exits 0.
- Its `data.needs_clarification` is 0, or each remaining marker is in front of the human.
