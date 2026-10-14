---
name: calibrate
description: Use to check Clef's decision points against labelled outcomes and propose threshold changes, even if shadow mode looks fine; explain each verdict and change thresholds only on the human's word. Do NOT use for proving work done (use `verify`) or turning one mistake into a check (use `correct`).
disable-model-invocation: true
argument-hint: "[D1..D7, or nothing for every point]"
---

# Calibrate

## Overview

Thresholds move on evidence and a human's yes. `anneal calibrate` scores each point and proposes a threshold as a diff; only `anneal calibrate --apply` writes it, and only after the human approves that diff.

- You MUST NOT edit `clef-thresholds.json` by hand or apply a proposal unasked; write it only with `anneal calibrate --apply --thresholds <file>` after the human says yes.
- You MUST NOT switch a point to `enforce` yourself: `anneal calibrate` lists the `clef_mode` value, and the human sets it.

## When to use

- Labels have accumulated (`anneal decide label`) and the human asks how Clef is doing.
- Not to prove work done (use `verify`), nor to turn one wrong call into a check (use `correct`).

## Verdicts

| Reason | Rule | What to do |
|---|---|---|
| `too_few_labelled` | fewer than 50 labelled decisions | label more with `anneal decide label <id> <outcome>` |
| `brier_above_bound` | Brier score over 0.15 | the probabilities are off; keep it in shadow |
| `ask_rate_at_or_above_bound` | asks 20% or more of the time | too many interruptions; keep it in shadow |
| none | `eligible-to-enforce` | the human may add `Dn=enforce` to `clef_mode` |

## Process

1. **Score.** Run `anneal calibrate --thresholds plugin/playbooks/clef-thresholds.json` (or the plugin's copy), with `--point Dn` when asked about one. It's working if it exits 0 and `data.points` holds each point asked for.
2. **Explain.** For each point give the verdict and its reasons from the table, the Brier score, the ask rate and labelled n, and say what a reliability bin far from its diagonal means. It's working if every verdict has its reason in plain words.
3. **Show the diff.** Print each `data.diff` entry as `-` its `before` and `+` its `after`, with the expected cost now and as proposed. With no entry, say the current thresholds cost least. It's working if the human sees the exact lines that would change.
4. **Ask.** Ask whether to apply, recommending yes only when the proposal is strictly cheaper and the point has 50 or more labelled decisions. It's working if the answer is an explicit yes or no.
5. **Apply.** On yes, run `anneal calibrate --apply --thresholds <same file>`, with the same `--point`. On exit 3 the file changed since; go back to step 1. It's working if `data.written` is true and `git diff` shows only those numbers.
6. **Hand over the mode.** For an `eligible-to-enforce` point, give the human the `clef_mode` value from `data.apply`. It's working if no mode was changed by you.

## Rationalizations

| Excuse | Reality |
|---|---|
| "The proposal is obviously better" | It is cheaper on the labels so far. The human decides whether those labels are representative. |
| "I'll just edit the number" | A hand edit skips the stale check and the strict re-read. Use `--apply`. |
| "It's eligible, so enforce it" | Eligible means the human may. The mode is theirs. |
| "49 labels is close enough" | The bar is 50 labelled decisions, not answered ones. |

## Red flags

- `clef-thresholds.json` changed with no `anneal calibrate --apply` run.
- A diff applied before the human answered.
- A point set to `enforce` by the agent.
- A verdict reported without its reason.

## Verification

- `anneal calibrate` exited 0 and every verdict was explained.
- The thresholds file changed only through `anneal calibrate --apply`, after an explicit yes.
- `git diff` of the thresholds file matches the approved `data.diff`.
