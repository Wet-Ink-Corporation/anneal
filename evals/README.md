# Evals

Skill evaluation suites, outside the packaged plugin. Tier 0 (static) and Tier 1 (lexical routing) run in CI from Release 1; Tier 2 (real routing) and Tier 3 (behaviour uplift) arrive in Release 6.

`evals/` lives at the repository root, never under `plugin/`, so it is not in the packaged plugin. A test (`repo_gates::the_packaged_plugin_holds_no_evals_directory`) fails if an `evals` directory appears anywhere under `plugin/`.

## Tier 0: static checks

The merge bar is zero errors from:

- `anneal lint skills plugin/skills`: the 21 skill design rules (`docs/skill-rules.md`).
- `claude plugin validate --strict` on the marketplace and on `plugin/`.
- The content-hash snapshot: a `lint skills --cache` run over `plugin/skills` reports exactly what a fresh run reports (`repo_gates::the_plugin_skills_lint_clean_and_the_snapshot_replays_the_fresh_result`).

## Tier 1: lexical routing

```sh
anneal eval routing [--skills plugin/skills] [--fixtures evals/routing]
```

Every model-invoked skill (one without `disable-model-invocation: true`) has a fixture `evals/routing/<skill>.yaml` with 8 to 10 **positive** prompts and 8 to 10 **near-miss** prompts. Each prompt is scored against every model-invoked skill by word overlap, in the style of addyosmani's `run-evals.js`:

- A skill's text is its directory name, its `name`, its trigger (the `description` up to its "Do NOT use for" clause, whose words point away from the skill) and its `when_to_use`.
- Words are lowercased runs of letters and digits, with apostrophes inside a word kept (`it's` is one word) and those at its ends trimmed, three characters or more, minus the stopwords of the `lint skills` overlap check. Each is cut to a light stem (`plans`, `planned` and `planning` all match `plan`).
- A skill's score is the number of distinct stems it shares with the prompt.

The merge bar is **100% rank-1**:

- A positive passes when its skill scores strictly higher than every other skill. A tie is a miss.
- A near miss passes when its owner scores strictly higher than the skill whose fixture lists it.

`repo_gates::the_plugin_skills_route_at_100_percent_rank_1` runs this over `plugin/skills` and `evals/routing`, so `cargo test` in CI fails on any miss or invalid fixture.

### Exit codes and report

| Code | Meaning |
|---|---|
| 0 | every fixture is valid and every prompt passes |
| 1 | below bound: at least one prompt misses |
| 2 | invalid fixtures or skills: a fixture does not parse, has too few or too many prompts, names an unknown skill or owner (or one whose `SKILL.md` does not parse), or a model-invoked skill has no fixture; or a `SKILL.md` does not parse |

A missing `--skills` or `--fixtures` directory is a usage error (2, one `ERROR` line, nothing on stdout).

The JSON envelope's `data` holds the counts (`skills`, `cases`, `passed`, `failed`, `problems`), the confusion table as a map (`confusion`: expected skill, then top-ranked skill, then count) and as aligned text (`table`). Rows are the expected skills (the fixture's skill for a positive, the owner for a near miss); columns are the top-ranked skill, plus `(tie)` and `(none)` (no shared word) when they occur. A near miss can pass off the diagonal when a third skill ranks first. Every miss and every problem is also a finding with its file and line.

```sh
anneal eval routing | jq -r '.data.table[]'
```

### Fixture schema

A fixture is a small subset of YAML, read by a hand-written reader so it needs no YAML dependency. Tier 2 (#40) runs the same files under the real router, so every file the reader accepts is also valid YAML that reads as the same strings: the reader rejects, and asks you to quote or rewrite, anything a YAML parser would read differently.

```yaml
# evals/routing/plan.yaml: prompts for the `plan` skill.
positives:
  - break the agreed spec into ordered tasks
  - "estimate the work: which tasks come first"
near_misses:
  spec:
    - write the acceptance criteria for the export feature
  diagnose:
    - the build breaks after the last task
```

- The file name is the skill's directory name plus `.yaml`. Other files in `evals/routing/` are ignored.
- Two top-level keys, each at most once: `positives:` (a list of prompts) and `near_misses:` (a map from owner skill to a list of prompts). Each must hold 8 to 10 prompts in total.
- Owners are skill names (lowercase letters, digits, hyphens), must be model-invoked skills whose `SKILL.md` parses, cannot be the fixture's own skill, and each lists at least one prompt.
- Items are `- prompt`, indented with spaces (no tabs), all items of one list at the same indentation and deeper than their key.
- Prompts are plain or quoted scalars with the frontmatter parser's rules: quote a prompt that contains `: `, ends with `:`, or starts with a YAML indicator (`` ` `` `@` `%` `*` `&` `!` `|` `>` `#` `[` `{`, or `- ` or `? `). Double quotes take only the `\"` and `\\` escapes (any other, such as `\t` or `é`, is rejected: YAML would decode it); single quotes take `''`.
- A plain prompt that YAML would read as something other than a string is rejected; quote it. That covers `~`, `null`, booleans (`true`, `false`, `yes`, `no`, `on`, `off`, `y`, `n`, in any case), numbers (`42`, `1_000`, `-3.5e2`, `0x1F`, `10:30`, `.inf`, `.nan`) and dates or timestamps (`2026-10-11...`). Longer phrases such as `3 tasks remain` or `null pointer crash` are plain text and need no quotes.
- Full-line `#` comments and blank lines are ignored.

### Seed fixtures

The first skills, `grill`, `spec` and `plan`, landed together with their fixtures (`evals/routing/grill.yaml`, `spec.yaml`, `plan.yaml`); each lists the other two as near-miss owners. `diagnose` (#25) came next with `evals/routing/diagnose.yaml`, whose near misses are owned by those three, and each of them gained near misses owned by `diagnose`. Entry skills that are not shipped yet (such as `build`) cannot be near-miss owners, since an owner must be a skill in the set; their fixtures add the near misses against them when they land. The bootstrap index is a session-start hook output, not a model-invoked skill, so it has no routing fixture of its own; its entry skills get theirs as they land. Each story that adds a model-invoked skill adds `evals/routing/<skill>.yaml` in the same change, and the run fails until it does. `build` (#24) landed with `build.yaml`, whose near misses are owned by `plan`, `spec`, `grill` and `diagnose`; `diagnose.yaml` gained a near miss owned by `build`, and `plan.yaml`, already at 10 near misses, gained none. `verify` (#26) landed with `verify.yaml`, whose near misses are owned by `diagnose`, `build`, `plan` and `grill`; every other fixture was already at 10 near misses, so none gained one owned by `verify`. `ship` (#35) landed with `ship.yaml`, whose near misses are owned by `verify`, `diagnose`, `build` and `plan`; `verify.yaml` gained two near misses owned by `ship` (its 9th and 10th), and `verify`'s Do NOT clause now names it as `(use \`ship\`)`.

The scorer itself is proved on test skill sets in `crates/anneal-cli/tests/fixtures/eval-routing/`: `pass/` routes every prompt, `fail/` has one positive that ranks the wrong skill first.

## Tier 2: real routing

The lexical proxy can lie. Tier 2 measures how often each skill really fires under Claude Code's router, with every sibling skill installed, and reports it as an interval.

```sh
anneal eval routing --router claude [--skills plugin/skills] [--fixtures evals/routing] \
  [--max-cost-usd 5] [--timeout 3600] [--concurrency 4] [--model <model>] \
  [--claude <path>] [--trust-plugin] [--changed-since <ref>]
```

It reads and checks the Tier 1 fixtures exactly as Tier 1 does; any problem exits 2 before anything runs or is spent. The plugin is the parent of `--skills` (`plugin/skills` is in `plugin`). Its directory is copied into a fresh temporary workspace, since `claude plugin eval` reads cases from below the plugin and writes results there; the repository is never written, and the workspace is removed at the end.

### How it runs

- **Cases.** Each positive prompt becomes one `claude plugin eval` case, written as `case.yaml` in JSON (YAML reads JSON as itself). A case has one `tool_used: Skill` grader per model-invoked skill, `fired-<skill>`, matching the Skill call's input `{"skill":"<plugin>:<skill>"}` with or without the plugin prefix, paired with an outcome grader, `answered` (the run ended with a non-empty reply), because a suite of `Skill` graders alone scores nothing. Runs have 2 turns and 180 s, and may use only the `Skill` tool.
- **The router.** `claude plugin eval <copy> --eval-dir anneal-routing-<round> --json <file> --runs 3 --ablation none --threshold 0 --max-cost-usd <left> --no-publish --no-scaffold --concurrency <n>`, plus `--model` and `--trust-plugin` when given. There is no baseline arm: without the plugin no anneal skill can fire. stdin is closed, stdout discarded and stderr kept in the workspace for the error message.
- **Negatives.** Every run of a skill's positive is a should-fire trial for that skill and a should-not trial for every other skill, so the negatives are the siblings' positives and the runs give a confusion matrix. Near misses are Tier 1's; they are not run here.
- **Sequential re-runs.** Each prompt runs 3 times. While its should-fire interval straddles 0.70 it runs 3 more, up to 12; once the interval is wholly at or above 0.70, or wholly below it, it stops. A perfect prompt settles at 9 of 9 (lower bound 0.701), a prompt that never fires at 0 of 3.
- **Cost cap.** Each batch gets `--max-cost-usd` minus what earlier batches cost, rounded down to the cent, and none starts with less than a cent left. `claude plugin eval` checks the ceiling before each run, so the overrun is bounded by the runs in flight.
- **Time limit.** `--timeout` (default 3600 s) bounds all batches together; a batch still running at the limit is killed and its runs are lost (agent runs it had already started end at their own 180 s timeout).
- **Errored runs.** `claude plugin eval` grades the trace of a run that ended in error too, so a run that fired a skill and then ran out of turns still counts as firing it. A run that ended in error with no skill fired (a timeout, a crash) is no evidence either way: it is counted, not a trial, but it counts toward a prompt's 12.

### The bar

The 95% Wilson score interval (z = 1.959964) of the runs pooled per skill:

- **should fire**: the skill's own prompts; the lower bound must be 0.70 or higher.
- **should not**: every other skill's prompts; the upper bound must be 0.20 or lower.

For example, 21 of 27 is 0.778 with a lower bound of 0.592, short of 0.70; 27 of 27 has 0.875. A skill passes when both bars are met.

### Exit codes and report

| Code | Meaning |
|---|---|
| 0 | every skill meets both bars, or `--changed-since` found no routing change (`data.skipped`) |
| 1 | below bound: a skill misses a bar |
| 2 | invalid fixtures or skills (nothing ran) |
| 3 | incomplete: the cost cap, the time limit or a failed later batch left a prompt unsettled |
| 4 | the router could not run: `claude` missing or failing without a result, no credentials, the plugin not loaded in the run, an unreadable result |

The envelope's `data` holds the counts (`skills`, `prompts`, `runs`, `errored`, `rounds`, `unsettled`), `cost_usd` and `max_cost_usd`, `complete`, `stop` (why it stopped early), `per_skill` (each skill's `should_fire` and `should_not`: bar, threshold, `fired`, `trials`, `interval`, `position`, plus `answered` and `passed`), `cases` (each prompt's runs and whether it settled) and the confusion table as a map (`confusion`) and as text (`table`): rows are the expected skill, columns the skill that fired, plus `(none)`, `(several)` and `(error)` when they occur. Each missed bar is a finding, as is an incomplete run.

```sh
anneal eval routing --router claude --trust-plugin | jq -r '.data.table[]'
```

### `--changed-since <ref>`

The suite costs money, so a pull request runs it only when routing could have changed: a model-invoked skill's `name`, `description` or `when_to_use` differs in the working tree from `<ref>`, a skill was added or removed, or `disable-model-invocation` flipped. Otherwise it exits 0 with `data.skipped: true` and runs nothing. A file that cannot be read on either side counts as a change.

### CI (pending)

Hooks execute outside the eval sandbox, so the suite runs in a container. The job belongs in `.github/workflows/`, which only a maintainer edits; this is the job to add, with the `ANTHROPIC_API_KEY` secret:

```yaml
name: routing-tier2
on:
  pull_request:
    paths: ["plugin/skills/**/SKILL.md", "evals/routing/**"]
jobs:
  tier2:
    runs-on: ubuntu-latest
    container: node:22-bookworm
    timeout-minutes: 90
    steps:
      - uses: actions/checkout@<sha> # with fetch-depth: 0, so the base is there
      - run: npm install -g @anthropic-ai/claude-code
      - run: curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
      - run: ~/.cargo/bin/cargo build --locked -p anneal-cli
      - run: >
          ./target/debug/anneal eval routing --router claude --trust-plugin
          --max-cost-usd 5 --changed-since "origin/${{ github.base_ref }}"
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
```

The path filter starts the job on any skill or fixture edit; `--changed-since` then spends nothing unless a description (or `when_to_use`, or the skill set) changed, and `--max-cost-usd 5` caps a run that does.

The Tier 2 code is proved without a model: the statistics against reference values, the stopping rule, the per-skill tally and the result reader against `claude plugin eval`'s result schema (version 1), and the whole command against a fake `claude` script (`crates/anneal-cli/tests/it/eval_routing_real.rs`).
