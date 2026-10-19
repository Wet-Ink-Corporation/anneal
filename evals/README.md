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
  [--max-cost-usd 25] [--timeout 3600] [--concurrency 4] [--model <model>] \
  [--claude <path>] [--trust-plugin] [--changed-since <ref>]
```

`claude` is found on `PATH`; on Windows an npm install is `claude.cmd`, which is not found that way, so pass `--claude <path to claude.cmd>`.

**Run it in a container.** `claude plugin eval` loads the plugin copy with its `hooks/hooks.json`, and those hooks run on the host, outside the eval sandbox, with your user's rights. That holds for a local run as much as for CI, so run Tier 2 in a container (as the CI job below does) unless you trust every hook of the plugin under test.

It reads and checks the Tier 1 fixtures exactly as Tier 1 does; any problem exits 2 before anything runs or is spent. The plugin is the parent of `--skills` (`plugin/skills` is in `plugin`). Its directory is copied into a fresh temporary workspace, since `claude plugin eval` reads cases from below the plugin and writes results there; the repository is never written, and the workspace is removed at the end.

### How it runs

- **Cases.** Each positive prompt becomes one `claude plugin eval` case, written as `case.yaml` in JSON (YAML reads JSON as itself). A case has one `tool_used: Skill` grader per model-invoked skill, `fired-<skill>`, matching the Skill call's input `{"skill":"<plugin>:<skill>"}` with or without the plugin prefix, paired with an outcome grader, `answered` (the run ended with a non-empty reply), because a suite of `Skill` graders alone scores nothing. Runs have 2 turns and 180 s, and may use only the `Skill` tool.
- **The router.** `claude plugin eval <copy> --eval-dir anneal-routing-<round> --json <file> --runs 3 --ablation none --threshold 0 --max-cost-usd <left> --no-publish --no-scaffold --concurrency <n>`, plus `--model` and `--trust-plugin` when given. There is no baseline arm: without the plugin no anneal skill can fire. stdin is closed, stdout discarded and stderr kept in the workspace for the error message.
- **Negatives.** Every run of a skill's positive is a should-fire trial for that skill and a should-not trial for every other skill, so the negatives are the siblings' positives and the runs give a confusion matrix. Near misses are Tier 1's; they are not run here.
- **Sequential re-runs.** Each prompt runs 3 times. While its should-fire interval straddles 0.70 it runs 3 more, up to 12; once the interval is wholly at or above 0.70, or wholly below it, it stops. A perfect prompt settles at 9 of 9 (lower bound 0.701), a prompt that never fires at 0 of 3.
- **Cost cap.** `--max-cost-usd` defaults to 25 US dollars, enough for a full run (wi-92bc1b). It is a cap on starting work, not a hard ceiling: each batch gets `--max-cost-usd` minus what earlier batches cost, rounded down to the cent, and none starts with less than a cent left. `claude plugin eval` checks the ceiling before each run, so the overrun is bounded by the runs in flight.
- **Time limit.** `--timeout` (default 3600 s) bounds the whole run from before the plugin copy, all batches together; a batch still running at the limit is killed and its runs are lost. Only `claude` itself is killed: sessions it started as child processes, if any, end at their own 180 s timeout and may still spend. The killed batch never reports its cost, so `data.cost_usd` is then a lower bound (`data.cost_is_lower_bound: true`), as it is after a later batch that failed once `claude` had run. When the first batch fails that way the run exits 4 and its `ERROR` line says the spend is unknown.
- **Checked results.** A case that reports more runs than its batch asked for is refused (an error on the first batch, a failed batch later, its runs dropped), so no prompt settles on runs no batch paid for. The workspace is made private to its owner (mode 0700 on unix) and each batch's stderr log is a fresh file, so another local account cannot plant files or links in it.
- **Errored runs.** `claude plugin eval` grades the trace of a run that ended in error too, and an error is judged by its kind. A run that ran out of turns (its error holds `Reached maximum number of turns (`) has decided routing, since the `Skill` call comes first or not at all, so it is a trial with whatever fired, nothing included. Any other error (a timeout, a crash) counts as a trial only when a skill fired; with none fired it is no evidence either way: it is not a trial, but it counts toward a prompt's 12 (weigh-in default wi-b4a3b0). `data.counted_errors` is how many runs ended in error and still counted. Telling the kinds apart rests on Claude Code's wording, so if it changes, out-of-turns runs fall back to the rule for other errors.

### The bar

The 95% Wilson score interval (z = 1.959964):

- **should fire, per prompt**: each of the skill's own prompts must settle with a lower bound of 0.70 or higher. A prompt that settles below it, or still straddles it after 12 runs, fails its skill, with one finding per prompt (`eval/routing/should-fire:<skill>:<file>:<line>`). The rate pooled over the skill's prompts is reported (`should_fire`) but is not the bar: a skill whose other prompts fire every time would otherwise hide a prompt that never does.
- **should not, pooled**: every other skill's prompts together; the upper bound must be 0.20 or lower. This bar cannot be per prompt: a single prompt's 0 of 12 has an upper bound of 0.242, so no one prompt could ever meet 0.20.

For example, 21 of 27 is 0.778 with a lower bound of 0.592, short of 0.70; 9 of 9 has 0.701. A skill passes when every one of its prompts meets the should-fire bar and its should-not bar is met.

### Exit codes and report

| Code | Meaning |
|---|---|
| 0 | every skill meets both bars, or `--changed-since` found no routing change (`data.skipped`); a run whose temporary workspace could not be removed still exits 0, with an `eval/routing/cleanup` finding naming why |
| 1 | below bound: a prompt misses its should-fire bar, or a skill its should-not bar |
| 2 | invalid fixtures or skills (nothing ran) |
| 3 | incomplete: the cost cap, the time limit or a failed later batch left a prompt unsettled |
| 4 | the router could not run: `claude` missing or failing without a result, no credentials, the plugin not loaded in the run, an unreadable result |

The envelope's `data` holds the counts (`skills`, `prompts`, `runs`, `errored`, `counted_errors`, `rounds`, `unsettled`), `cost_usd`, `cost_is_lower_bound` and `max_cost_usd`, `complete`, `stop` (why it stopped early), `per_skill` (each skill's `should_fire` and `should_not`: bar, threshold, `fired`, `trials`, `interval`, `position`, plus `prompts_met`, `prompts_failed`, `answered` and `passed`), `cases` (each prompt's runs and whether it settled) and the confusion table as a map (`confusion`) and as text (`table`): rows are the expected skill, columns the skill that fired, plus `(none)`, `(several)` and `(error)` when they occur. Each failing prompt and each missed should-not bar is a finding, as is an incomplete run.

```sh
anneal eval routing --router claude --trust-plugin | jq -r '.data.table[]'
```

### `--changed-since <ref>`

The suite costs money, so a pull request runs it only when routing could have changed: a model-invoked skill's `name`, `description` or `when_to_use` differs in the working tree from `<ref>`, a skill was added or removed, or `disable-model-invocation` flipped. Otherwise it exits 0 with `data.skipped: true` and runs nothing. A skill file that cannot be read (or whose frontmatter does not parse) on one side only counts as a change; one unreadable on both sides compares equal. A working-tree skill that does not read already stops the run at Tier 1's checks (exit 2).

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

## Tier 3: behaviour uplift

Routing says a skill fires; Tier 3 asks whether firing it changes what the agent does. Each entry skill (`grill`, `spec`, `plan`, `build`, `diagnose`, `verify`, `ship`) has a regression suite and a capability suite of cases under `evals/uplift/<skill>/`, and each case runs 3 times with the skill and 3 times without it.

```sh
anneal eval uplift [--skills plugin/skills] [--suites evals/uplift] [--skill <name>]... \
  [--check] [--baseline <plugin dir>] [--max-cost-usd 25] [--timeout 3600] \
  [--concurrency 4] [--model <model>] [--claude <path>] [--trust-plugin] [--changed-since <ref>] \
  [--allow-gated-tools] [--allow-scaffolds]
```

`--check` reads and checks the suites and reports each skill's coverage (`data.coverage`); it runs nothing, and a run flag beside it is a usage error. `cargo test` runs it over this directory (`tests/it/repo_gates.rs`), so CI refuses an invalid case or an entry skill without both suites. `--skill` (repeatable) runs one skill's suites; naming a skill with no suite directory is a usage error.

### Layout and case schema

```
evals/uplift/<skill>/regression/<case>.json   # passes with the skill nearly every time
evals/uplift/<skill>/capability/<case>.json   # passes rarely for now, and should rise
evals/uplift/fixtures/<name>/                 # files a case's runs start with (`fixture`)
```

`fixtures/` holds the fixtures and is never a skill's suite directory (a skill named `fixtures` cannot have suites). `<skill>` must be a skill directory under `--skills`, with at least one of the two suites and no `--` in its name (`--` separates the parts of a case's name, `<skill>--<suite>--<case>`); `<case>` is 1 to 64 of `a-z0-9-`. Other files (such as this README) are ignored, symlinks are refused, and at most 512 cases are read. A case is strict JSON (unknown fields refused):

```json
{
  "behaviour": "guard",
  "pressure": "authority",
  "seed": "anneal #13 review (blocker): a build session edited source with no approved plan.",
  "prompt": "Add a `--json` flag to the `report` command. There is no plan for it yet.",
  "max_turns": 10,
  "timeout_seconds": 300,
  "allowed_tools": ["Skill", "Read", "Grep", "Glob"],
  "graders": [
    {"type": "regex", "name": "names_the_plan", "target": "last_message", "pattern": "\\bplan\\b", "flags": "i"},
    {"type": "llm", "name": "judge", "criteria": "The reply does not start implementing ..."}
  ]
}
```

- `behaviour`: `outcome`, scored by its pass rate, or `guard`, a line the skill must hold under pressure, which must pass all 3 runs with the skill (pass^3).
- `pressure` (optional): a pressure fixture whose sentence is added after a blank line at the end of the prompt. `authority`: "The tech lead says to skip the tests for this one." `time`: "We ship in 5 minutes, so be quick." `sunk-cost`: "We already wrote it, so just merge it."
- `seed`: the real failure the case was seeded from (an issue and the review finding or hook message), with no secrets, home-directory paths or email addresses.
- `max_turns` 1 to 50 (default 10), `timeout_seconds` 30 to 1800 (default 300).
- `allowed_tools`: from `Skill`, `Read`, `Grep`, `Glob`, `Write`, `Edit`, `Bash` (default the first four; `Skill` is required, or the skill could never load). `claude plugin eval` withholds `Write`, `Edit` and `Bash` unless they are granted, so a run passes `--allow-tools` with exactly the gated tools its cases name. No other tool, so no case reaches the network. The grant comes from the case files alone, so every case that names a gated tool is listed in `data.granted_tools` (`{case, tools}`) and named in one `eval/uplift/granted-tools` finding, in `--check` and in a run: review those cases before running a suite you did not write. A run grants them only with `--allow-gated-tools` (Ryan, 2026-10-13): without it, a run whose selected cases (after `--skill` and `--changed-since`) name a gated tool is refused before anything is copied, spent or run, exiting 2 with one `ERROR` line that names the flag and the case files (the first 3, then a count), and nothing on stdout. A selection with no such case needs no flag. `--check` lists the grants as before and, like every run flag, refuses `--allow-gated-tools` beside it. The seed suites need the flag for a full run: every `build` and `diagnose` case, and `plan/regression/plan-before-build.json`, name `Write`, `Edit` and `Bash`, since those skills run commands (`anneal repro record`, `anneal lint plan`, the tests) to do their job.
- `fixture` (optional): the name of a directory under `fixtures/` beside the skill directories (1 to 64 of `A-Za-z0-9._-`, not starting with `.`, so no path separator and no `..`), whose files each run of the case starts with. Every `claude plugin eval` run starts in an empty working directory, so a case that needs a repository (a crate to edit, docs to read) names one. The fixture must be a plain directory (not a symlink) holding at least one file, or a symlink to a file or directory, within 10,000 entries; it may hold relative symlinks into the repository, such as `docs -> ../../../../docs`, which the run copies dereferenced. A symlink that dangles, resolves outside the repository (git's top level for the suites directory; only when the suites are in no git repository, the parent of their nearest `evals/` ancestor; with neither, every symlink is refused) or back into a directory it is inside is refused. A case whose fixture fails these checks is a case problem (exit 2), in `--check` too. The case is written to `case.yaml` at schema 1.1 with `context: {scaffold_script: scaffold.sh}`, and `scaffold.sh` beside it copies the fixture into the run's workspace (`cp -RL '<absolute fixture path>/.' .`, the path single-quoted) and commits it as a fresh git repository (`git init -q`, `git add -A`, `git -c user.name=eval -c user.email=eval@invalid -c commit.gpgsign=false commit -qm fixture`); a scaffold gets no `EVAL_*` variables, so the path is written into it. A case without a fixture is written as before, at schema 1.0 with no `context`.
  The seed suites have two fixtures. `py-app` is a small Python command-line tool (`report`, `tail`, `deleted-tests`, `delete`) whose tests run with the standard library alone (`python3 -m unittest`), so a run needs no `pytest`. It holds four seeded bugs for the `diagnose` cases, each reproducible with one command: `tail 0` prints every finding (`items[-0:]`), `delete` deletes a file whose audit line could not be written (the `OSError` is swallowed), `deleted-tests` misses a deleted test whose path has a space (git's trailing TAB is kept on the `---` line), and `report` reads `app.ini` from the current directory rather than from the log file's. It has no `src/duration.py`, which `build/regression/red-before-green` asks to create test-first. `anneal-repo` links this repository, so its bugs are already fixed: only cases that read it (spec, verify, `diagnose/capability/out-of-turns`) use it, never a case that asks for a fix. A case that needs only reasoning (`ship`, `grill/capability/bandless-question`) names no fixture.
  **Cost and safety**: a scaffold script runs outside the eval sandbox, as you, with your rights, before each run of the case (3 per arm). A run passes `--scaffold` only for a batch holding a fixture case, `--no-scaffold` otherwise, and runs scaffolds only with `--allow-scaffolds`, as gated tools need `--allow-gated-tools`: without it, a run whose selected cases name a fixture is refused before anything is copied, spent or run, exiting 2 with one `ERROR` line that names the flag and the case files (the first 3, then a count). Every fixture case is listed in `data.scaffolded` (`{case, fixture}`) and named in one `eval/uplift/scaffolded` finding, in `--check` and in a run: review the cases and their fixtures before running a suite you did not write. `--check` refuses `--allow-scaffolds` beside it. A scaffold that fails or takes over 120 seconds scores its run 0 (`claude plugin eval`'s `scaffold failed`).
- `graders`: `claude plugin eval`'s own types (case schema 1.1, read from Claude Code 2.1.296), written through unchanged: `tool_order`, `regex`, `file_exists` and `tool_used` first, `llm` last (an `llm` grader before a deterministic one is refused), unique names, and at least one grader that checks the outcome rather than only `tool_used`. Its `baseline` grader and `arm` field are not accepted: each arm is its own run. A run passes when every one of its case's graders is reported passed: a grader the result leaves out failed, and a result naming a grader the case does not have, or one grader twice in a run, is refused.

Graders check behaviour, not routing: a `tool_used: Skill` grader would pass with the skill and fail without it by construction, inflating the uplift.

### Arms

The with arm runs every case against a temporary copy of the whole plugin. The baseline, by default, is one batch per skill against a copy with that skill's directory removed from the plugin's skills directory (the last component of `--skills`, whatever it is called), so its hooks and sibling skills stay in place and only the skill is missing; `claude plugin eval --ablation with-without` was not used because it strips the whole plugin, hooks included. `--baseline <plugin dir>` instead runs every case against another plugin: an older version, or this plugin without a scaffolding hook (below). Its results are checked under the name in its own `.claude-plugin/plugin.json`; a `--baseline` with no readable manifest is a usage error (2) before anything is copied, and one that cannot be copied fails with 70, the workspace removed. No copy keeps an `anneal-uplift-*` directory left in the source plugin, so only the selected cases run. Each arm is a single-arm `claude plugin eval <copy> --eval-dir anneal-uplift-<n> --runs 3 --ablation none --threshold 0 --max-cost-usd <left> [--allow-tools ...] --no-publish --scaffold|--no-scaffold --concurrency <n>` in the workspace `anneal-eval-uplift-*` under the temporary directory, removed at the end; the repository is never written. Only the cases a batch asked for are kept from its result. A result that lists a case twice, or a run naming a grader its case lacks or one grader twice, is refused as a case reporting too many runs is: an error (4) on the first batch, a failed batch later with its reported cost counted and its runs dropped. With no case selected (none under `--suites`, or none changed since `--changed-since`) nothing runs and the run is reported skipped (exit 0).

Every plugin copy (the with copy, each skill-removed copy and the `--baseline` copy) gets the running `anneal` binary at `libexec/<target>/anneal` (`anneal.exe` on Windows), executable on unix, where `<target>` is the one the plugin's launcher (`plugin/libexec/anneal`) tries first on this host (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `aarch64-apple-darwin`, `x86_64-apple-darwin` or `x86_64-pc-windows-msvc`): the launcher's release download fails inside the eval sandbox, so without it the runs' hooks and skills would find no `anneal`. `data.libexec_target` names the target; on a host the launcher does not know none is placed, `data.libexec_target` is `null` and an `eval/uplift/libexec` finding says so. A binary that cannot be found or copied is a workspace error, as a failed plugin copy is: 70 for the with and `--baseline` copies, made before any batch, and a failed batch for a skill-removed copy made later.

Tier 3 shares Tier 2's adapter, and with it Tier 2's safeguards. The workspace is private to its owner on unix (0700) and each batch's stderr log is a new file. `--timeout` counts from before the plugin copies, a skill-removed copy's time included. A case that reports more runs than its batch asked for is refused: an error (4) on the first batch; on a later batch, a failed batch that ends the run, its runs dropped and its reported cost kept. A later batch that fails or is killed makes `cost_usd` a lower bound. A first batch that fails after `claude` started says in its `ERROR` line that its spend is unknown, unless its result reported its cost and was then refused, when the line says what it reported spending. A refused batch whose cost is a lower bound keeps the run's `cost_usd` a lower bound too.

### Statistics and the bars

A run that ends in error is no trial, except one that ran out of turns, which is graded as Tier 2 grades it. Per skill, over all its cases' runs pooled:

- **Uplift**: the difference of pass rates, with-arm minus baseline, with its 95% Newcombe hybrid score interval (method 10, built on Wilson). The bar is a lower bound of 0 or higher. 6 of 6 against 0 of 6 has a lower bound of 0.448; 3 of 3 against 3 of 3 is -0.561 to 0.561, so a case that passes either way cannot carry a skill.
- **Guards**: every `guard` case passes all 3 runs with the skill (pass^3); one failed or missing run fails the skill.
- **Regression**: a regression suite regresses when the 95% Wilson upper bound of its with-arm pass rate is below 0.90 (1 of 3 is 0.79), so the suite must stay near 100%.

Listed but never failing the run:

- **Non-discriminating assertions**: a grader that passed in 90% or more of a case's graded baseline runs tells the skill from nothing; each is listed in `data.non_discriminating` and as a finding (`eval/uplift/non-discriminating:<file>:<grader>`). Rewrite or drop it.
- **Graduates**: a capability suite whose every with-arm run passed is listed in `data.graduates` (by skill); move its cases to `regression/`, since a capability that always holds is now a regression to guard.

### Exit codes and report

| Code | Meaning |
|---|---|
| 0 | every skill meets every bar; `--check` with valid suites; or `--changed-since` found no change (`data.skipped`) |
| 1 | below bound: an uplift lower bound below 0, a guard not pass^3, or a regressed regression suite |
| 2 | invalid suites, cases or skills (nothing ran), or a usage error (gated tools without `--allow-gated-tools`, and fixtures without `--allow-scaffolds`, among them) |
| 3 | incomplete: the cost cap, the time limit or a failed later batch stopped the run, or a case has fewer than 3 runs in an arm |
| 4 | `claude plugin eval` could not run its first batch |

`data` holds `mode` (`check` or `run`), `skills`, `cases`, `runs_per_arm`, `runs`, `errored`, `cost_usd`, `cost_is_lower_bound`, `max_cost_usd`, `baseline` (`skill-removed` or `plugin`, with `baseline_plugin`), `complete`, `stop`, `skipped`, `problems`, `coverage`, `per_skill` (each skill's `with`, `base`, `uplift`, `uplift_met`, its suites and `guards_failed`), `results` (each case's counts), `non_discriminating`, `graduates`, `granted_tools`, `scaffolded` and `libexec_target`. Findings: `eval/uplift/suite:<file>`, `uplift:<skill>`, `guard:<file>`, `regressed:<skill>`, `non-discriminating:<file>:<grader>`, `granted-tools`, `scaffolded`, `libexec`, `incomplete` and `cleanup`.

### Cost cap

`--max-cost-usd` defaults to 25 US dollars (wi-92bc1b, as Tier 2). The with arm gets at most half the cap, so the baseline always has the rest to compare against; each later batch gets what is left, rounded down to the cent, and none starts below a cent; the overrun is bounded by the runs in flight. A full run is 27 cases x 3 runs x 2 arms = 162 runs.

### `--changed-since <ref>`

Unlike Tier 2, any change counts: a skill runs when anything under its skill directory (its body included) or its suites changed in the working tree since `<ref>`, committed or not, or an untracked file appeared there. With none changed it exits 0 with `data.skipped: true`. A listing too long to read whole runs every skill.

### Seed cases

The 27 cases were seeded from 26 real failures found in this repository's local Claude Code transcripts (reviewer blocker and major findings, and Stop-hook feedback, from stories #10 to #40), since #27's dogfood findings do not exist yet: build 7, verify 5, diagnose 5, spec 4, plan 2, ship 2, and one `grill` failure written as a regression and a capability case. Prompts and seeds were redacted: no secrets, home-directory paths or email addresses. Add a case for each new real failure a skill should have prevented, with the failure in `seed`.

### Re-running after a model change

Scaffolding hooks (tagged in `plugin/hooks/hooks.metadata.json`) exist to make up for what a model does not do unprompted; a stronger model may not need them. When the default model changes:

1. Copy the plugin and remove the scaffolding hook from the copy: its entry in `hooks/hooks.json` and its row in `hooks/hooks.metadata.json`.
2. Run the suites of the skills the hook serves against the copy as the baseline, under the new model:

   ```sh
   anneal eval uplift --skill <skill> --baseline <copy> --model <new model> --trust-plugin
   ```

   The with arm is the plugin with the hook; the baseline is the plugin without it. Add `--allow-gated-tools` when the skill's cases name `Write`, `Edit` or `Bash` (`build`'s do), after reviewing them; without it the run is refused before anything is spent.
3. If no skill's uplift lower bound is 0 or higher, the hook adds nothing measurable under the new model: retire it in a pull request that records the run (the envelope, the model and the date) in the hook's metadata row and the decisions log. If any skill's lower bound is 0 or higher, keep it.

### CI (pending)

As for Tier 2, the job belongs in `.github/workflows/`, which only a maintainer edits; this is the job to add, with the `ANTHROPIC_API_KEY` secret:

```yaml
name: uplift-tier3
on:
  pull_request:
    paths: ["plugin/skills/**", "evals/uplift/**"]
jobs:
  tier3:
    runs-on: ubuntu-latest
    container: node:22-bookworm
    timeout-minutes: 90
    steps:
      - uses: actions/checkout@<sha> # with fetch-depth: 0, so the base is there
      - run: npm install -g @anthropic-ai/claude-code
      - run: curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
      - run: ~/.cargo/bin/cargo build --locked -p anneal-cli
      - run: >
          ./target/debug/anneal eval uplift --trust-plugin --allow-gated-tools
          --max-cost-usd 25 --changed-since "origin/${{ github.base_ref }}"
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
```

The path filter starts the job on any skill body or suite edit; `--changed-since` then runs only the skills whose files or suites changed, and `--max-cost-usd 25` caps the run. The job passes `--allow-gated-tools` because the seed `build` suite names `Bash` (`build/regression/red-before-green.json`), so a change to `build` would otherwise be refused; the cases it grants are the ones the pull request's review read, and each run lists them in `data.granted_tools` and the `eval/uplift/granted-tools` finding. `pull_request` runs from forks get no secrets, so a fork's new case never runs with the key. A maintainer who would rather not grant tools in CI drops the flag: a change selecting a gated case then fails the job (exit 2) without spending anything.

The Tier 3 code is proved without a model: the Newcombe interval against reference values, the case reader, suites layout, tally, batch loop and report with fakes of the `Evaluator` port, and the whole command against a fake `claude` script (`crates/anneal-cli/tests/it/eval_uplift.rs`).
