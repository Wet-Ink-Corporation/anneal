---
name: orchestrate
description: Use to run an approved plan's ready tickets across subagents, even when fanning out looks obvious; park one-way calls in weigh-in and rule on the rest. Do NOT use for one task (use `build`) or splitting work into tickets (use `plan`).
disable-model-invocation: true
argument-hint: "[tickets directory or --tracker github --repo owner/name]"
---

# Orchestrate

## Overview

Rulings, not stalls. Fanning out costs about 15 times the tokens of one session, so it runs only over independent tickets, each with a brief, a fresh verifier and five fix rounds at most.

- You MUST fan out only when `anneal orchestrate fan-out` exits 0 (3 or more independent tickets); on exit 3, build the tickets one at a time.
- You MUST spawn every subagent with a `Brief:` line from `anneal brief build`; the `brief` hook denies any other spawn.
- You MUST ask `anneal decide --point D2 --question noul` about every judgment call, and park only when it says `park` with `data.enforce` true, through `anneal orchestrate park`.
- You MUST record every other call with `anneal orchestrate rule`: what, why, cost if wrong.
- You MUST NOT write `weigh-in` marker blocks for a parked call; `anneal orchestrate park` already raised it.

## When to use

- A plan is approved and `anneal dag frontier` reads its tickets.
- Not for one task (use `build`), nor for turning a spec into tickets (use `plan`).

## Process

1. **Gate the fan-out.** Run `anneal orchestrate fan-out` with the tracker flags of `anneal dag frontier`. It's working if you fan out on exit 0 and work one ticket at a time on exit 3.
2. **Brief each ticket.** Split tickets `dag frontier` flags over budget, then run `anneal brief build --ticket <id> --base <ref>`. It's working if `anneal lint brief` passes for each.
3. **Implement.** Spawn one implementer per ticket with `Brief: .anneal/briefs/<id>.md` and nothing of yours. It's working if the `brief` hook let every spawn through.
4. **Verify fresh.** For each ticket spawn a new verifier with the review package only, never the implementer's report or claims. It's working if the verifier read the diff, not a summary.
5. **Loop.** On a failed verification run `anneal round start --ticket <id>` and do what `data.next` says. It's working if no ticket passes five rounds; after five, park it (step 6).
6. **Decide each call.** Ask D2 with the call as state. On `park` with `data.enforce` true, run `anneal orchestrate park --ticket <id> --q <question> --default <option> --flip <fact> --at <path:line> --resolve-by <today + 7 days> --decision <data.id>` and move on. Otherwise (`rule`, shadow, no answer, or park exit 4) choose, then run `anneal orchestrate rule --ticket <id> --what --why --cost --decision <data.id>`. It's working if every call ends parked or recorded, never asked in chat.
7. **Report.** List each ticket's verdict, rulings and parked `wi-` items. It's working if the human can settle each parked item in `/weigh-in` unaided.

## Rationalizations

| Excuse | Reality |
|---|---|
| "Two tickets, but fanning out is still faster" | Below three the token cost outweighs the time saved. |
| "The implementer already checked it" | A verifier that reads the claims checks the claims. |
| "I'll just ask the human in chat" | That stalls the run. Park it with a default, or rule. |
| "Clef is in shadow, so nothing to log" | Shadow means you rule; record the ruling. |

## Red flags

- A spawn without a `Brief:` line, or with your notes pasted in.
- A verifier shown the implementer's final message.
- A call ruled in chat with no D2 decision.
- A `weigh-in` marker block written for a call you parked.

## Verification

- `anneal ledger query --type weighin_parked` and `--type orchestrate.ruled` list each call.
- `weighin open --repo` shows each parked item with its default.
- Each ticket has a verifier verdict within five rounds.
