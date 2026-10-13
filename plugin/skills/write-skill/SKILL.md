---
name: write-skill
description: Use to write or change a skill test-first, even if the change looks like a wording fix; watch a case fail with no guidance, then pass Tiers 0 to 3 before merge. Do NOT use for turning a correction into a check (use `correct`) or building code (use `build`).
disable-model-invocation: true
argument-hint: "[skill name and the failure it prevents]"
---

# Write a skill

## Overview

A skill is code for an agent: no failing case, no skill. Each change starts from a real failure, is shown to fail with no guidance, and merges only once Tiers 0 to 3 pass (`evals/README.md`).

- You MUST see a case fail in the no-guidance control before writing the skill text: the baseline arm of `anneal eval uplift`, run without the skill.
- You MUST NOT merge until `anneal lint skills plugin/skills --index docs/bootstrap-index.md` has no error and `anneal eval uplift` meets its bars for the skill.
- You MUST also pass `anneal eval routing` and `anneal eval routing --router claude` for a model-invoked skill.

## When to use

- A skill is new, or its text, frontmatter or references change.
- Not to turn one correction into a check (use `correct`), nor to build code (use `build`).

## The tiers

Tier 0 is `anneal lint skills` and `claude plugin validate --strict plugin`; Tier 1 `anneal eval routing`; Tier 2 `anneal eval routing --router claude`; Tier 3 `anneal eval uplift`. Their bars, what each applies to and what they cost are in `references/tiers.md`. Tiers 2 and 3 spend money: ask the human before each run.

## Process

1. **Seed the case.** Take a real failure (a review finding, a gate block) and write it as a case under `evals/uplift/<skill>/regression/` or `capability/`, with its `seed`, deterministic graders first and a guard for each line the skill must hold. It's working if `anneal eval uplift --check` exits 0.
2. **Red: no guidance.** With the human's approval, run `anneal eval uplift --skill <name>`; for a new skill the baseline is the plugin without it. It's working if the baseline arm fails the case.
3. **Write the skill.** Use skill-creator's shape: a `description` opening with `Use`, saying `even if`, ending `Do NOT use for ... (use <sibling>)`; then `## Overview` (hard rules first), `## When to use`, `## Process`, `## Rationalizations`, `## Red flags`, `## Verification`; detail in `references/`. It's working if the body stays within 600 words.
4. **Tier 0.** Run the Tier 0 commands. It's working if both exit 0 with no error.
5. **Tiers 1 and 2.** For a model-invoked skill, add `evals/routing/<skill>.yaml`, run Tier 1, then ask before Tier 2. A user-invoked skill (`disable-model-invocation: true`) has no routing fixture. It's working if both bars are met.
6. **Green: Tier 3.** Ask, then run `anneal eval uplift --skill <name>`. It's working if `data.per_skill` shows `uplift_met` and no failed guard.
7. **Refactor.** Cut words that no case needs, then repeat steps 4 to 6. It's working if every tier still passes.

## Rationalizations

| Excuse | Reality |
|---|---|
| "It's only wording" | Wording is the skill. Wording changes behaviour, so it gets a case. |
| "The model already does this" | Then the control passes and the skill adds nothing. Prove the gap first. |
| "Evals cost too much" | One capped run costs less than a skill that misfires every session. |
| "Lexical routing passed" | Tier 1 is a proxy; Tier 2 measures the real router. |

## Red flags

- Skill text written before any case failed without it.
- A grader that checks only that the skill fired.
- A Tier 2 or 3 run started without the human's approval.
- Merging with a Tier 0 error or a missed bar.

## Verification

- Tier 0 exits 0; for a model-invoked skill, Tiers 1 and 2 meet their bars.
- `anneal eval uplift --skill <name>` meets its bars, and its baseline arm failed the seed case.
