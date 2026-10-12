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
