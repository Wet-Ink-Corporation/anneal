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

Routing says a skill fires; Tier 3 asks whether firing it changes what the agent does. Each entry skill (`grill`, `spec`, `plan`, `build`, `diagnose`, `verify`, `ship`) has a regression suite and a capability suite of cases under `evals/uplift/<skill>/`, and each case runs 3 times with the skill and 3 times without it (`--runs` asks for 3 to 20).

```sh
anneal eval uplift [--skills plugin/skills] [--suites evals/uplift] [--skill <name>]... \
  [--case <case>] [--runs 3] [--no-baseline] [--check] [--baseline <plugin dir>] [--max-cost-usd 25] [--timeout 3600] \
  [--concurrency 4] [--model <model>] [--claude <path>] [--trust-plugin] [--changed-since <ref>] \
  [--allow-gated-tools] [--allow-scaffolds] [--work-dir <dir>] [--keep-results <dir>]
```

`--check` reads and checks the suites and reports each skill's coverage (`data.coverage`); it runs nothing, and a run flag beside it is a usage error. `cargo test` runs it over this directory (`tests/it/repo_gates.rs`), so CI refuses an invalid case or an entry skill without both suites. `--skill` (repeatable) runs one skill's suites; naming a skill with no suite directory is a usage error. `--case` runs one case, named by its file under the suites directory (`build/regression/plan-before-source.json`), that file's path (made absolute against the current directory, with no `..` or link resolved), or its name (`build--regression--plan-before-source`); a `--case` naming no case (of the `--skill`s given) is a usage error, it is allowed with `--check`, and refused with `--changed-since`. `--runs` asks for 3 to 20 runs per case in each arm (default 3). `--no-baseline` runs the with arm alone, with the whole cost cap, to confirm a watched guard (see Guards below); it is refused with `--baseline` and with `--check`, as `--runs` is with `--check`.

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

- `behaviour`: `outcome`, scored by its pass rate, or `guard`, a line the skill must hold under pressure, which may slip at most one run with the skill (see Guards below).
- `pressure` (optional): a pressure fixture whose sentence is added after a blank line at the end of the prompt. `authority`: "The tech lead says to skip the tests for this one." `time`: "We ship in 5 minutes, so be quick." `sunk-cost`: "We already wrote it, so just merge it."
- `seed`: the real failure the case was seeded from (an issue and the review finding or hook message), with no secrets, home-directory paths or email addresses.
- `max_turns` 1 to 50 (default 10), `timeout_seconds` 30 to 1800 (default 300).
- `allowed_tools`: from `Skill`, `Read`, `Grep`, `Glob`, `Write`, `Edit`, `Bash` (default the first four; `Skill` is required, or the skill could never load). `claude plugin eval` withholds `Write`, `Edit` and `Bash` unless they are granted, so a batch passes `--allow-tools` with the gated tools its cases name, and `claude plugin eval` grants those to every case of the batch; each case's `case.yaml` therefore withholds the ones it does not name (`execution.disallowed_tools`), so every case gets exactly its own tools in both arms, whichever batch it runs in (r4's with arm ran all cases in one batch and gave each the union). No other tool, so no case reaches the network. The grant comes from the case files alone, so every case that names a gated tool is listed in `data.granted_tools` (`{case, tools}`) and named in one `eval/uplift/granted-tools` finding, in `--check` and in a run: review those cases before running a suite you did not write. A run grants them only with `--allow-gated-tools` (Ryan, 2026-10-13): without it, a run whose selected cases (after `--skill` and `--changed-since`) name a gated tool is refused before anything is copied, spent or run, exiting 2 with one `ERROR` line that names the flag and the case files (the first 3, then a count), and nothing on stdout. A selection with no such case needs no flag. `--check` lists the grants as before and, like every run flag, refuses `--allow-gated-tools` beside it. The seed suites need the flag for a full run: every `build` and `diagnose` case, and `plan/regression/plan-before-build.json`, name `Write`, `Edit` and `Bash`, since those skills run commands (`anneal repro record`, `anneal lint plan`, the tests) to do their job; the `grill` cases, `plan/capability/linear-duplicate-check.json` and `spec/regression/mode-ledger-only.json` name `Write` and `Bash` (grill writes its design tree and runs `anneal grill frontier`), and every `verify` case names `Bash`, to run `anneal task done` and the tests fresh (Tier 3 r7).
- `fixture` (optional): the name of a directory under `fixtures/` beside the skill directories (1 to 64 of `A-Za-z0-9._-`, not starting with `.`, so no path separator and no `..`), whose files each run of the case starts with. Every `claude plugin eval` run starts in an empty working directory, so a case that needs a repository (a crate to edit, docs to read) names one. The fixture must be a plain directory (not a symlink) holding at least one file, or a symlink to a file or directory, within 10,000 entries; it may hold relative symlinks into the repository, such as `docs -> ../../../../docs`, which the run copies dereferenced. A symlink that dangles, resolves outside the repository (git's top level for the suites directory; only when the suites are in no git repository, the parent of their nearest `evals/` ancestor; with neither, every symlink is refused) or back into a directory it is inside is refused. A case whose fixture fails these checks is a case problem (exit 2), in `--check` too. The case is written to `case.yaml` at schema 1.1 with `context: {scaffold_script: scaffold.sh}`, and `scaffold.sh` beside it copies the fixture into the run's workspace (`cp -RL '<absolute fixture path>/.' .`, the path single-quoted) and commits it as a fresh git repository on branch `main`, so `floor-guard` finds a base (`git init -q -b main`, git 2.28 or later, `git add -A`, `git -c user.name=eval -c user.email=eval@invalid -c commit.gpgsign=false commit -qm fixture`); a scaffold gets no `EVAL_*` variables, so the path is written into it. A case without a fixture is written as before, at schema 1.0 with no `context`.
  The seed suites' fixtures follow. `py-app` is a small Python command-line tool (`report`, `tail`, `deleted-tests`, `delete`) whose tests run with the standard library alone (`python3 -m unittest`); its README says so and that `pytest` is not installed, since runs that reached for `pytest` never saw a red test (Tier 3 r6). Each Python fixture has a `.gitignore` for `__pycache__/` and `*.pyc`, so bytecode the tests write is not counted as a source edit after the last green run by the completeness Stop hook (Tier 3 r6). It holds four seeded bugs for the `diagnose` cases, each reproducible with one command: `tail 0` prints every finding (`items[-0:]`), `delete` deletes a file whose audit line could not be written (the `OSError` is swallowed), `deleted-tests` misses a deleted test whose path has a space (git's trailing TAB is kept on the `---` line), and `report` reads `app.ini` from the current directory rather than from the log file's. `anneal-repo` links this repository, so its bugs are already fixed: only cases that read it (spec, `verify/regression/decision-matches-code`, `diagnose/capability/out-of-turns`) use it, never a case that asks for a fix. It links `evals/routing` but not this README, whose Tier 3 notes are the grading of its own cases (Tier 3 r9). `decision-matches-code` changes it in its `setup`: a `python3 -c` edit leaves `crates/anneal-ledger/src/store/schema.rs` opening the ledger under WAL (`conn.pragma_update(None, "journal_mode", "WAL")`, `JOURNAL_MODE` `"wal"`) as an uncommitted change, against the decisions-log row "never runs in WAL", so the change the prompt names is in the tree to find and read (Tier 3 r9). `cargo` cannot run in the eval sandbox (rustup cannot download the pinned toolchain there), so a case on it is graded on what the code shows, not on a run: `out-of-turns` grades the ranked hypotheses checked against the code, and does not ask for the 2-turn limit to be raised, since at `HEAD` an out-of-turns run already counts as a trial (Tier 3 r8). `cache-app` is a capped cache whose README cites a test, `the_cache_never_grows_past_its_cap`; the test exists and fails, because `put` evicts only once the cache is already over its capacity, so the cache holds one entry too many. Grep finds the test; only a fresh `python3 -m unittest` shows it red (`verify/regression/claimed-test-exists`, re-seeded in Tier 3 r8, when the test was missing and a negative answer needed no run). `snapshot-app` is a snapshot reader whose 1 MiB cap reads at most `MAX_BYTES` and so never refuses a larger file, with a test at the cap and none past it (`verify/capability/snapshot-cap-tested`). `calibrate-app` proposes a decision threshold from a labelled set on which 0.60 and 0.62 both cost 14, and its `propose` takes a candidate that only ties (`verify/capability/proposal-cost`). Those two replace prompts that carried the answer ("adds the check but no test", "both cost 14"), so neither arm can answer without reading or running the fixture (Tier 3 r8). `py-app-planned` is `py-app` (linked) with a spec, its design tree and a plan of six independent tasks (`docs/plans/hardening.md`). The fixture records nothing in the ledger; the `build` process cases confirm the design tree and approve the plan in their `setup` (`anneal grill confirm --tree docs/design/hardening.json --quote "Yes, that's the plan, go ahead."`, then `anneal plan approve docs/plans/hardening.md --actor human --via bash`), so their runs start with `anneal plan check` passing; their prompts say the plan is approved and ask for one task, test-first (`build/regression/red-before-green` asks for T6, a new `src/duration.py`). `build/regression/plan-before-source` has no `setup` and asks for a task with no approval. Each task names its files, test command and blockers only, with no red/green steps, so the arm without `build` is not handed the process (Tier 3 r7). The build process cases grade the trace (`target: trace`), not tool order: `test_first` and `red_run` see a test or source file written by Write, Edit or Bash (`cat > ...`, `open(..., 'w')`, `sed -i` and the like), and `task_done` needs a green `anneal task done` result, so a refused edit or a red `task done` no longer passes (Tier 3 r7). `test_first` and `red_run` step through the trace a line (one message) at a time, `^(?:(?![^\n]*?X)[^\n]*\n)*?(?:(?!X)[^\n])*?Y`, not a character at a time, `^(?:(?!X)[\s\S])*?Y`: `claude plugin eval` runs on Bun, whose regex engine gives up on the character-at-a-time form over a trace of a few hundred kilobytes and reports no match, without an error, so in r8 every `red_run` failed and some `test_first` passed in the baseline where Node's engine reads them the other way. The two forms match the same traces; check a new pattern of this kind under Bun (Tier 3 r8). `redact-app` is a command-line redactor before the one-line `-u` fix of `verify/capability/redaction-leak`, with the fix beside it as `fix.patch` and no test for it, so a test can be seen red before the fix and green after (Tier 3 r7). `clef-notes` describes the Clef decision points beside a link to `crates/anneal-cli/playbooks/clef-thresholds.json`, for `grill/capability/bandless-question`. `ticket-graph` is a ticket-graph loader with a ready spec for refusing duplicate ids, for `plan/capability/linear-duplicate-check`. `blank` holds only a `.gitkeep`: a case that needs no files (the reasoning cases, and those that ask for a test list or a plan) names it, so its run's worktree is a repository of its own. A case with no fixture runs in `claude plugin eval`'s `home/cwd`, whose git root is `home/`, so the Stop hook counted tool dotfiles there (`.gitconfig`, `.config/git/ignore`, `.npm/_logs`) as changed source and rewrote the reply the graders read (Tier 3 r4).
  **Cost and safety**: a scaffold script runs outside the eval sandbox, as you, with your rights, before each run of the case (3 per arm). A run passes `--scaffold` only for a batch holding a fixture case, `--no-scaffold` otherwise, and runs scaffolds only with `--allow-scaffolds`, as gated tools need `--allow-gated-tools`: without it, a run whose selected cases name a fixture is refused before anything is copied, spent or run, exiting 2 with one `ERROR` line that names the flag and the case files (the first 3, then a count). Every fixture case is listed in `data.scaffolded` (`{case, fixture}`) and named in one `eval/uplift/scaffolded` finding, in `--check` and in a run: review the cases and their fixtures before running a suite you did not write. `--check` refuses `--allow-scaffolds` beside it. A scaffold that fails or takes over 120 seconds scores its run 0 (`claude plugin eval`'s `scaffold failed`).
- `setup` (optional, fixture cases only): commands the scaffold runs in the run's workspace after its commit, in order, for state a fixture cannot carry, such as ledger events under `.git/anneal/`. Each command is an argument vector, never a shell string: `[["anneal", "plan", "approve", "docs/plans/p.md", "--actor", "human", "--via", "bash"]]`. At most 8 commands, each 1 to 32 words, the first not blank, each word at most 512 bytes with no control character; `setup` without a `fixture`, or a command breaking these rules, is a case problem (exit 2, in `--check` too, naming the command by its position from 1). Each word is written into `scaffold.sh` single-quoted, after `PATH='<plugin copy>/libexec':"$PATH"`, so a bare `anneal` is the plugin copy's launcher, which runs the binary the run's hooks run (the running `anneal`, placed at `libexec/<target>/anneal`). The script runs under `set -eu`, so a failing command fails the scaffold and its run scores 0 (`scaffold failed`). Setup commands run outside the sandbox with your rights like the rest of the scaffold: review them with the case.
- `graders`: `claude plugin eval`'s own types (case schema 1.1, read from Claude Code 2.1.296), written through unchanged: `tool_order`, `regex`, `file_exists` and `tool_used` first, `llm` last (an `llm` grader before a deterministic one is refused), unique names, and at least one grader that checks the outcome rather than only `tool_used`. Its `baseline` grader and `arm` field are not accepted: each arm is its own run. A run passes when every one of its case's graders is reported passed: a grader the result leaves out failed, and a result naming a grader the case does not have, or one grader twice in a run, is refused.

Case and grader fixes after Tier 3 r8:

- **Order is graded from the trace, not by a judge.** An `llm` grader with `focus: trace` sees only the first 12 and the last 12 lines of the trace (Claude Code 2.1.316), and a trace is mostly `stream_event` lines (400 of 441 in one r8 run), so a judge cannot see the order of tool calls. The `diagnose` cases `enforce-log-failure`, `reproduce-first` and `worktree-from-file` therefore check it with `repro_before_edit`, a trace regex: a red outcome recorded by anneal (`"outcome":"red"` in the result of `anneal repro record` or `anneal task done`) appears in a tool result before the first source edit, and a source edit follows. An edit is a Write or Edit of `src/app/*.py`, or a Bash write to one (`sed -i`, `>`, `tee`, `open(..., 'w')`, `cp` and the like, as `build/regression/plan-before-source` reads them), so a fix made with `sed -i` after a red reproduction now counts (r8's `worktree-from-file-2`). Their judges grade what the reply reports, so a short reply forced by the completeness Stop hook is judged on its content rather than on retelling the order. `worktree-from-file`'s judge no longer asks for ranked hypotheses when reading the code shows the cause, and its `file_path` regex also takes "the directory of the log file" and "the log's directory".
- **`verify/capability/redaction-leak`** passed its old `regression_test` regex on "I didn't add a regression test". It now needs a Bash command that writes a test feeding `curl -u` (`curl_u_test`) and a unittest run that fails followed by one that passes (`red_then_green`: `FAILED (failures=...` in a tool result, then `Ran N tests ... OK` in a later one). `max_turns` is 16, room for a red run, the patch and a green run.
- **`grill/capability/bandless-question`**'s `recommendation` regex is r7's `recommend` again. The phrasings added after r7 ("I'd treat ...") credited a baseline reply that recommends nothing (r8's `4-without-grill` run 1); the judge still reads the recommendation itself.
- **`plan/regression/plan-before-build`** asked to "start building", which routes to `build` in both arms, so it could not show `plan`'s effect. It now asks to plan the export spec so it can be built next, and keeps its plan graders (`lints_plan`, `plan_commands`) with `build/regression/plan-before-source`'s three no-source-edit graders: the line it holds is a linted plan, every task keeping a test command under "skip the tests", with no build before approval. `build/regression/plan-before-source` already guards building without an approved plan.

Case and grader fixes after Tier 3 r9:

- **`anneal-repo` no longer links `evals/README.md`.** Its Tier 3 notes held `out-of-turns`' own grading note, and two baseline runs read and cited it. The fixture keeps `evals/routing`.
- **`verify/regression/decision-matches-code`** seeds the WAL change it asks about, as an uncommitted edit in `setup` (above). Every r9 run reported the change missing from the checkout, so the case graded a description rather than code.
- **`diagnose/regression/reproduce-first`**: `reproduce` also takes "failed before (or without) the fix (or change)", the wording of all three r9 with-arm replies, which ran a red `anneal repro record` before their first edit. Its judge counts a unit test of `last()` as the reproduction and takes "failed before the fix and passes now" as the order, which `repro_before_edit` checks.
- **`diagnose/capability/tab-in-paths`** gets the shared `repro_before_edit` trace grader. Without it a fix written before any reproduction passed, and the case did not discriminate (r9: 3/3 in both arms).
- **`verify/capability/redaction-leak`**: `red_then_green` also takes `FAILED (failures=...` and then `Ran N tests ... OK` in one Bash result, since all three r9 with-arm runs ran the red and the green run in one command (`git stash` of the fix, the tests, `git stash pop`, `anneal task done`).
- **`verify/capability/snapshot-cap-tested`**: `boundary` also takes `MAX_BYTES + 1`, which every r9 reply wrote. Its judge no longer accepts reading the code in place of a fresh run past the cap, and says a run that never reached a test (`No module named pytest`) shows nothing.
- **`verify/capability/proposal-cost`**: the judge's test clause is now the `tie_test` regex on the reply (a tie test, or a test where a candidate ties the current threshold), since r9's judge failed two replies that asked for one. Both arms ask for one, so the case still does not discriminate; that needs a tie test seen red in the trace.

Case and grader fixes after Tier 3 r10:

- **`verify/regression/claimed-test-exists`**: the judge grades the verdict and its evidence only, not a fix the reply proposes. r10's with-3 met every clause (a fresh red run, "the ticket shouldn't be closed") and was failed, likely for an aside on how to fix the cache.
- **`verify/capability/redaction-leak`**: the judge no longer passes a reply that only asks for the test ("asks for, or" is gone), so it matches `curl_u_test` and `red_then_green`, which need a written test seen red, then green.
- **`verify/regression/decision-matches-code`** gets `fresh_run`, a `tool_used` Bash grader matching `cargo (?:test|nextest)`: r10 with 3/3, baseline 0/3. It rewards only the attempt: rustup cannot download the pinned toolchain in the eval sandbox, so no test runs, and the with arm says so. A red guard test (`the_ledger_runs_under_truncate_and_never_creates_wal_or_shm`) needs a cached toolchain in the sandbox.
- **`verify/capability/snapshot-cap-tested`** gets `past_cap_run`, a trace regex for a Bash command that builds a file past the cap (`MAX_BYTES + 1`, `1048577` or `1024 * 1024 + 1`), the judge's run clause made deterministic: r10 with 0/3, baseline 2/3, exactly the runs that wrote and read a file one byte over the cap. It matches no other r10 trace.
- **`verify/capability/proposal-cost`** gets no `cites_contract` (`README.md:6` in the reply, with 3/3, baseline 0/3): it would grade the skill's own citation style rather than the outcome (Ryan).

Case and grader fixes after Tier 3 r11:

- **`verify/capability/redaction-leak`**: `red_then_green` needs the red before the first `git apply`, stepping a line at a time: each line before the red has no `git apply`, or is one Bash command that runs `unittest` before `git apply` (r11's with-2). r11's with-3 applied `fix.patch` before writing its test, and its only red came from another test on the patched code; had its green subset run not been filtered through `grep -o`, the old pattern would have passed it. Grades are unchanged: r10 with 0/3, baseline 0/3; r11 with 1/3, baseline 0/3.
- **`build/regression/plan-before-source`** gets `red_test_next`, a regex on the reply for the judge's last clause (the work then starts with a failing test: "failing test", "red test", "test first"), so a miss is attributable. It matches the judge run for run: r10 with 3/3, baseline 0/3; r11 with 2/3, baseline 0/3.
- **`build/regression/red-before-green`**: the judge drops `focus: trace`, whose 25-line window showed no tool call, so it judged near-identical replies both ways (r10 votes PFF, PPF, FFF; r11 one pass). It now grades what the reply reports: a test covering `5m`, `2h` and a ValueError, run and seen failing before `src/duration.py` was written, and passing now. `test_first` and `red_run` keep grading the order.
- **`plan/regression/plan-before-build`** gets `plan_lint_green`, a trace regex for a green `anneal lint plan` result (`task_done`'s form): r10 with 3/3, baseline 1/3; r11 with 3/3, baseline 0/3. A green lint means every task names its files, one test and valid blockers, so the judge, which reads only the reply, no longer asks for them. It fails a reply that offers to drop the tests or replace them with manual checks.
- **`spec/regression/mode-ledger-only`** gets no `store_only` regex; the judge stays its only store grader.

Case and grader fixes after Tier 3 r12:

- **`grill/capability/bandless-question`** gets `max_turns` 16. The skill's path on this fixture (load the skill, read, write the design tree, run `anneal grill frontier`, reply) takes about 9 turns, so the default 10 left at most one to spare: r11's tree-keeping runs used 10 and 9, and r12's with-1 was cut off at 11 with no reply after a frontier that already asked the right question. Its `numbered` regex, `^[ \t]*(?:#{1,6}[ \t]+)?(?:\*\*|__)?1[.)](?:\*\*|__)?[ \t]`, takes bold or heading numbering (`**1. ...**`) and uses `[ \t]` in place of `\s`, so it cannot step across lines: r11 with 3/3, baseline 2/3 (was 1/3); r12 with 2/3 (was 1/3: with-3), baseline 2/3 (was 1/3). The baseline gain is honest: r11's and r12's base-2 asked and recommended rightly (`recommendation` and judge both passed), so the baseline passes rise to 1/3 in each round. No `exempts_record_only` grader is added: it would match the judge but not raise discrimination.
- **`diagnose/regression/reproduce-first`**: `reproduce` takes either word order, past or present tense, with a window that steps over the `.` in a file name but stops at a sentence end (`\bfail(?:ed|s)\b ... \bbefore (?:the|my|any) (?:fix|change|edit)\b`, or the reverse), so r12's with-1 "**Before the fix:** the new `tests/test_tail.py` failed" passes and the baseline's "haven't seen it fail before the fix" still misses: r11 with 3/3, baseline 0/3; r12 with 3/3 (was 2/3), baseline 0/3. Its judge grades only what `reproduce` and `repro_before_edit` cannot see: "before the edit" and "before the fix" mean the same, the reproduction may come after the cause and the fix, and a side note such as negative counts now returning nothing does not fail it.
- **`verify/capability/proposal-cost`**: `tie_test`'s windows step over a decimal point (`0.62 ties with 0.60, so ...`), and it also takes a tie test's name (`test_tie_...`, which `\btie` cannot match inside): r11 with 3/3, baseline 3/3; r12 with 3/3 (was 2/3: with-1 wrote and ran `test_tie_with_current_keeps_current` red), baseline 3/3.
- **`verify/capability/redaction-leak`**: the judge is a checklist (a test feeding `curl -u user:<password>` through `redact`, red before `fix.patch`, green after it, run fresh), and a yes for the reported form that also names forms still leaking passes. r12's three with runs did all three and drew votes PPP, FPF and FFP, likely on whether an opening "Yes" with forms still leaking called it fixed on the patch alone. `curl_u_test` and `red_then_green` are unchanged.
- **`build/regression/plan-before-source`**: the judge grades what its regexes cannot see, that the reply does not agree to skip, defer or shrink the test because the tech lead said so, and settles the waiver: saying that only the human can drop a test, by changing the plan and approving it again, is not agreeing to skip it, since test changes are the human's decision (H-16). All six r11 and r12 with replies make that offer, and the judge split on them (r12 6 passing votes of 9).
- **`verify/capability/proposal-cost`** gets `tie_red_run`, a trace regex that steps a line at a time for a tool result holding the tie test's red, `AssertionError: 0.62 != 0.6`: the skill's red run of the claim's case, which the baseline, computing the costs by hand, never makes. Over r10 to r12 the case scored with 8/9, baseline 8/9, since every grader accepted a reply that only asks for a tie test. On the recorded traces it gives r11 with 2/3, baseline 0/3; r12 with 2/3, baseline 0/3. It does not also require `FAIL: test_` on the line, which r11's with-2 cut off with `tail -8`. The judge leaves the test to `tie_test` and `tie_red_run`.
- **`verify/capability/snapshot-cap-tested`** gets `cap_red_run`, a trace regex for a tool result holding `SnapshotTooLarge not raised`, a test one byte past the cap seen red, where `past_cap_run` also passes a probe by hand: r11 with 1/3 (with-3, the only run that wrote the test), baseline 0/3; r12 with 0/3, baseline 0/3. Over r10 to r12 the case scored with 4/9, baseline 6/9, both arms probing by hand, so without this grader it measures nothing; it pays off only with the `verify` skill asking for a test of the claim's case before any verdict.

Case and grader fixes after Tier 3 r13:

- **`build/regression/red-before-green`**: the judge grades only the red-first claim, which no trace grader can see in the reply: the test was run and seen failing before `src/duration.py` was written (a missing module counts as that red), and it passes now. Naming the test file or its test count beside what the code returns for `5m`, `2h` and bad input is enough; the reply need not list the test's cases. The coverage clause measured wording only: every run's test asserts `5m`, `2h` and a ValueError (a coverage regex passed 18/18 r11 to r13 traces, both arms), yet the judge split on near-identical replies (r12 and r13 with 13 passing votes of 18; r13 with-2 PFF). `test_first` and `red_run` keep grading the order and are unchanged.
- **`verify/capability/redaction-leak`**: `red_then_green` becomes `red_then_rerun`, a red unittest run before any `git apply`, then a Bash command that runs `git apply`, then a later unittest result, green or red (a red and a green in one result, r13 with-1's form, still passes). The case's patch fixes `-u user:pw` but not `-uuser:pw`, so a run that also tests the forms the patch misses (r13 with-2) could never show a whole-suite green; whether the reported test passes after the patch is the judge's clause (3). It stays a guard. On the recorded traces: r11 with 1/3, baseline 0/3; r12 with 3/3, baseline 0/3; r13 with 2/3 (was 1/3: with-2), baseline 0/3. r13 with-3, a hand check, then the patch, then the test, still fails.

Case and grader fixes after Tier 3 r14:

- **`verify/capability/redaction-leak`**: `red_then_rerun` ties the red result to its own Bash command by `tool_use_id`. A Bash command with no `git apply` before it (or one that runs `unittest` before `git apply`) whose own result is `FAILED`, then either that result going green in the same command, or a later `git apply` command and a unittest result from a different call. A parallel batch writes both `tool_use` lines before either result, so r14's with-1, which sent its red run and `git apply` in one message (the ledger has the red first), failed the line-ordered pattern; an apply issued before the red command still fails. On the recorded traces: r10 with 0/3, r11 1/3, r12 3/3, r13 2/3, unchanged; r14 with 3/3 (was 2/3: with-1); baseline 0/3 in every round.
- **`diagnose/regression/reproduce-first`**: the judge also excuses running one test module rather than the whole suite (in this fixture `tests/test_cli.py` is the whole suite) and which file the new test is in. r14's with-1 recorded red before its edit and drew FFF on a reply that said "I ran only `tests.test_cli`".
- **`build/regression/plan-before-source`**: the judge's waiver allowance covers any wording that routes dropping or waiving the test through a plan change the human approves, whoever is said to want the test gone, and calling the test small or naming its cases is not shrinking it. r14's with-3 ("If the tech lead wants the test requirement waived, that needs a change to the plan, which you would approve") drew FFF with every regex grader passing.
- The line-ordered order graders of `reproduce-first` (`repro_before_edit`) and `red-before-green` (`test_first`, `red_run`) were re-graded on the r10 to r14 traces, both arms: every verdict reproduces the recorded one, and no recorded FAIL hides a red run issued before the first source write behind a parallel batch, so they are unchanged.

Case and grader fixes after Tier 3 r15:

- **`plan/regression/plan-before-build`**: the judge does not count saying that only the human and the lead can decide to drop tests as offering to drop them, and counts saying it will run `anneal plan approve` once the human says yes as asking for approval, whether or not the command is named. r15's with-1 kept every `Test:`, got a green `anneal lint plan` and drew FFF on a reply that said "If the lead wants tests dropped, that decision needs to be made with you"; r14's with-2 drew FFP on the same move. No base grade changes: the deterministic graders fail every base run.
- **`diagnose/capability/enforce-log-failure`**: the judge grades only the outcome: with `APP_AUDIT_LOG` in a missing directory, `app delete` exits non-zero and keeps the file, seen in a run after the fix. It does not fail a reply for adding no test, not running the existing suite or naming anneal commands; `reproduce` and `repro_before_edit` already grade the reproduction and its order. r15's with-2 and r10's with-1 were correct runs the judge failed (FPF and FFF); `repro_before_edit` fails every base trial from r10 to r15, so the judge added no discrimination.

Case and grader fixes after Tier 3 r16:

- **`build/regression/plan-before-source`**: `no_done_claim` skips "it's done" or "it is done" after when, once, until, after, if or before, so r16's with-2 ("Tell me when it's done"), a reply that stopped for approval, passes. "It's done.", "T4 is now complete" and "I've implemented T4" still fail. Re-graded on the r11 to r16 kept results, only that trial changes.

Case and grader fixes after Tier 3 r17:

- **`verify/capability/redaction-leak`**: `red_then_rerun` no longer counts a `git apply` that changes nothing in the repo as the patch applied before the red: `git apply --check` (a dry run), or a `git apply` after a `cd` into a `mktemp -d` or `$TMPDIR` copy. The apply after the red must be a real one; main accepted a `--check` there. r17's confirmation traces 2 and 3 each got a red on the unpatched repo, applied the patch and got a green, and failed only on an earlier dry run or scratch-copy apply; trace 10, a hand check with no test, still fails every grader. On the recorded traces: r10 to r16 unchanged in both arms, r17 with 2/3 and finish 3/3 unchanged, baseline 0/3 in every round; the r17 confirmation becomes 9/10 (was 7/10), which counts only once a fresh confirmation repeats it.

Case and grader fixes after the r17 re-confirmation:

- **`verify/capability/redaction-leak`**: `red_then_rerun`'s same-command branch accepts the rerun green or red, as the cross-call branch already did. The command must run `unittest`, then a real `git apply` (not `--check`), then `unittest` again, with no apply before the first run, and its own result must show `FAILED` followed by a second `Ran N tests` line. The old branch (red then `OK` in one result) never looked for an apply in the command, so red, `git apply --check`, green passed. The re-confirmation's trace 8 wrote tests, ran them red, applied `fix.patch` and reran in one command, still red for the two forms the patch misses. It passes now. On the recorded traces: r10 to r17 unchanged in both arms, the r17 confirmation unchanged, the re-confirmation 10/10 (was 9/10: trace 8), baseline 0/3 in every round.

Case and grader fixes after Tier 3 r19:

- **`diagnose/regression/reproduce-first`** and **`verify/regression/claimed-test-exists`**: each judge grades only what the reply alone can show. For reproduce-first that is a reproduction that failed before the fix and passes after it, and a count of 0 now printing nothing. For claimed-test-exists it is the verdict (the bound is not shown to hold, or the ticket stays open) because `test_the_cache_never_grows_past_its_cap` fails. Run order and freshness stay with the trace graders (`repro_before_edit`, `fresh_run`, `red_seen`). Every r19 miss in both was judge-only on a correct reply (confirmations 7/10 each), as were reproduce-first's r18 misses and claimed-test-exists's r16 miss. With the narrowed judges, 10-run checks scored claimed-test-exists 9/10 and reproduce-first 10/10. No regex changed.

Case and grader fixes after the r20 confirmations:

- **`build/regression/plan-before-source`**: `red_test_next` also accepts "watch it fail first" and kin: "watch" or "see" with "it", "them" or "the test", then "fail" or "go red", and "fail(s/ing) first". The r20 confirmation's with-4 ("I'll still write `tests/test_report_out.py` and watch it fail first") and the r20 round's with-3 named the red test as the next step and failed only this regex. On every kept plan-before-source reply from r10 to the r20 confirmation (74 runs), only those two change, both to a pass; base runs fail the `no_source_*` graders either way. Checked under Node and Bun.

Every trace regex here steps a line at a time and was checked under Node and under Bun (`~/.bun/bin/bun`) on the 162 r8 traces, with the same verdict on each; the r9 changes were checked the same way on the 162 r9 traces, the r10 changes on the 162 r10 traces, the r11 changes on the r10 and r11 traces of their cases, the r12 changes on the r11 and r12 traces of their cases, the r13 changes on the r11 to r13 traces of their cases, the r14 change to `red_then_rerun` on the r10 to r14 traces of its case (with and baseline), the r17 change to it on the r10 to r17 traces of its case, the r17 confirmation included, and the change after the re-confirmation on those traces and the re-confirmation's, each unchanged grader reproducing its recorded verdict.

Graders check behaviour, not routing: a `tool_used: Skill` grader would pass with the skill and fail without it by construction, inflating the uplift.

### Arms

The with arm runs every case against a temporary copy of the whole plugin, `with/` in the workspace (not `plugin/`: the anneal repository has `plugin/` at its root, and a run that read `${CLAUDE_PLUGIN_ROOT}` took the workspace for it, Tier 3 r6). The baseline, by default, is one batch per skill against a copy with that skill's directory removed from the plugin's skills directory (the last component of `--skills`, whatever it is called), so its hooks and sibling skills stay in place and only the skill is missing; `claude plugin eval --ablation with-without` was not used because it strips the whole plugin, hooks included. `--baseline <plugin dir>` instead runs every case against another plugin: an older version, or this plugin without a scaffolding hook (below). Its results are checked under the name in its own `.claude-plugin/plugin.json`; a `--baseline` with no readable manifest is a usage error (2) before anything is copied, and one that cannot be copied fails with 70, the workspace removed. No copy keeps an `anneal-uplift-*` directory left in the source plugin, so only the selected cases run. Each arm is a single-arm `claude plugin eval <copy> --eval-dir anneal-uplift-<n> --runs 3 --ablation none --threshold 0 --max-cost-usd <left> [--allow-tools ...] --no-publish --scaffold|--no-scaffold --concurrency <n>` in the workspace `ae-<6 hex>` under the work directory (`--work-dir`), removed at the end; the repository is never written. Only the cases a batch asked for are kept from its result. A result that lists a case twice, or a run naming a grader its case lacks or one grader twice, is refused as a case reporting too many runs is: an error (4) on the first batch, a failed batch later with its reported cost counted and its runs dropped. With no case selected (none under `--suites`, or none changed since `--changed-since`) nothing runs and the run is reported skipped (exit 0).

Every plugin copy (the with copy, each skill-removed copy and the `--baseline` copy) gets the running `anneal` binary at `libexec/<target>/anneal` (`anneal.exe` on Windows), executable on unix, where `<target>` is the one the plugin's launcher (`plugin/libexec/anneal`) tries first on this host (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `aarch64-apple-darwin`, `x86_64-apple-darwin` or `x86_64-pc-windows-msvc`): the launcher's release download fails inside the eval sandbox, so without it the runs' hooks and skills would find no `anneal`. `data.libexec_target` names the target; on a host the launcher does not know none is placed, `data.libexec_target` is `null` and an `eval/uplift/libexec` finding says so. A binary that cannot be found or copied is a workspace error, as a failed plugin copy is: 70 for the with and `--baseline` copies, made before any batch, and a failed batch for a skill-removed copy made later.

Tier 3 shares Tier 2's adapter, and with it Tier 2's safeguards. The workspace is made in `--work-dir` (made if missing), else in `$TMPDIR` when it is set (on Linux, only when it is outside the directories below), else in `<anneal home>/tmp`, else, with no home set, in the system's temporary directory (a relative directory is taken from the current directory), and each batch's `claude` runs with `TMPDIR` set to the workspace's `t/`, where `claude plugin eval` makes its runs' sandbox roots. On Linux its sandbox denies Bash every write under `/tmp`, `/var/tmp`, `/dev/shm` and `/run/shm`, run directories included, so a run made there can write nothing from Bash, the anneal ledger included (Tier 3 r4); a run whose selected cases name Bash with its workspace there gets an `eval/uplift/read-only-sandbox` finding. The work directory must also be short (Tier 3 r5): on Linux each run's Bash sandbox binds its bridge sockets at `<TMPDIR>/claude-eval-XXXXXX/tmp/claude-socks-<16 hex>.sock` (Claude Code 2.1.314; the `http` socket is a byte shorter), and a Unix socket path holds at most 107 bytes, so the `claude` child's `TMPDIR` may be at most 49 bytes once resolved. The workspace is `ae-<6 hex>` and that `TMPDIR` its `t/`, so the `TMPDIR` is the work directory plus 12 bytes, and the work directory may be at most 37 (`/root/.anneal/tmp` is 17 and `/home/<user>/.anneal/tmp` fits for a user name of up to 19 bytes; for a longer one pass a short `--work-dir`). Tier 2's workspace keeps its `anneal-eval-routing-<6 hex>` name, as its children get no `TMPDIR` in it. A run whose selected cases name Bash with a longer one gets a `WARN eval:` line on stderr before anything is copied or spent, and an `eval/uplift/socket-path` finding; it still runs, but Bash fails in every run ("Failed to create bridge sockets"). macOS makes its runs under `/tmp/e-XXXXXX` whatever the `TMPDIR`, so it is not checked there. `--keep-results <dir>` copies each batch's result (every run's grader verdicts, the `llm` judges' included) to `<dir>/result-<batch>-<arm>.json` (`with`, `without-<skill>` or `baseline`) and each of its runs' traces (the result's `tracePath`) to `<dir>/traces/<batch>-<arm>/<case>-<run>.jsonl` before the workspace is removed, overwriting a file of that name. On Linux its batches pass `--keep-temp`, without which `claude plugin eval` removes each run's directory, trace included, as soon as the run ends; the kept directories are inside the workspace and go with it (a kept directory `claude` sealed for a user other than root is opened again to be removed). A trace named outside the workspace is left where it is; one named inside that resolves outside it (through `..` or a link) or is not a file is an `eval/uplift/keep-results:<file>` finding. A result or trace it cannot write is an `eval/uplift/keep-results:<file>` finding, and its batch's runs and cost still count. The workspace is private to its owner on unix (0700) and each batch's stderr log is a new file. `--timeout` counts from before the plugin copies, a skill-removed copy's time included. A case that reports more runs than its batch asked for is refused: an error (4) on the first batch; on a later batch, a failed batch that ends the run, its runs dropped and its reported cost kept. A later batch that fails or is killed makes `cost_usd` a lower bound. A first batch that fails after `claude` started says in its `ERROR` line that its spend is unknown, unless its result reported its cost and was then refused, when the line says what it reported spending. A refused batch whose cost is a lower bound keeps the run's `cost_usd` a lower bound too.

### Statistics and the bars

A run that ends in error is no trial, except one that ran out of turns, which is graded as Tier 2 grades it. Per skill, over all its cases' runs pooled:

- **Uplift**: the difference of pass rates, with-arm minus baseline, with its 95% Newcombe hybrid score interval (method 10, built on Wilson). The bar is a lower bound of 0 or higher. 6 of 6 against 0 of 6 has a lower bound of 0.448; 3 of 3 against 3 of 3 is -0.561 to 0.561, so a case that passes either way cannot carry a skill.
- **Guards** (Ryan, 2026-11-07): each `guard` case gets a band from its runs with the skill, `data.results[].guard_band`. A run asked for that errored or never came counts as not passing. A guard may slip one run and is then watched; two slips break it: at 3 runs, 3 of 3 is `held`, 2 of 3 `watch` and 1 or 0 of 3 `broken`, and at any `--runs` n, n is held, n-1 watch and fewer broken. Only a broken guard fails the skill (`per_skill[].guards_failed`). A watched guard is listed in `per_skill[].guards_watch` and `data.watch` (`{skill, case, passed, runs, confirm}`) and as an `eval/uplift/watch:<file>` finding, both holding the command that confirms it: `anneal eval uplift --case <file> --no-baseline --runs 10` (plus `--allow-gated-tools` and `--allow-scaffolds` when the case needs them, and the round's `--skills`, `--suites`, `--claude`, `--model`, `--timeout`, `--concurrency`, `--max-cost-usd`, `--work-dir` and `--trust-plugin`). A confirmation runs the with arm alone, computes no uplift (`uplift_met` is `null`, with an `eval/uplift/no-baseline` finding) and holds each guard when it passes 9 in 10 of the runs asked for (`held`, else `failed-confirmation`, which fails the skill). `guard_held` is true when the band is `held`.
- **Regression**: a regression suite regresses when the 95% Wilson upper bound of its with-arm pass rate is below 0.90 (1 of 3 is 0.79), so the suite must stay near 100%.

Listed but never failing the run:

- **Non-discriminating assertions**: a grader that passed in 90% or more of a case's graded baseline runs tells the skill from nothing; each is listed in `data.non_discriminating` and as a finding (`eval/uplift/non-discriminating:<file>:<grader>`). Rewrite or drop it.
- **Graduates**: a capability suite whose every with-arm run passed in a round of whole suites (not a `--no-baseline` confirmation, and not under `--case`) is listed in `data.graduates` (by skill); move its cases to `regression/`, since a capability that always holds is now a regression to guard.

### Exit codes and report

| Code | Meaning |
|---|---|
| 0 | every skill meets every bar; `--check` with valid suites; or `--changed-since` found no change (`data.skipped`) |
| 1 | below bound: an uplift lower bound below 0 (never in a `--no-baseline` confirmation), a broken guard or one that failed its confirmation, or a regressed regression suite; a watched guard alone exits 0 |
| 2 | invalid suites, cases or skills (nothing ran), or a usage error (gated tools without `--allow-gated-tools`, fixtures without `--allow-scaffolds`, a `--case` naming no case, a `--runs` outside 3 to 20, `--no-baseline` with `--baseline` or `--check`, and `--case` with `--changed-since`, among them) |
| 3 | incomplete: the cost cap, the time limit or a failed later batch stopped the run, or a case has fewer than the runs asked for in an arm (in the with arm only, with `--no-baseline`) |
| 4 | `claude plugin eval` could not run its first batch |

`data` holds `mode` (`check` or `run`), `skills`, `cases`, `runs_per_arm`, `runs`, `errored`, `cost_usd`, `cost_is_lower_bound`, `max_cost_usd`, `baseline` (`skill-removed`, `plugin`, with `baseline_plugin`, or `none` for `--no-baseline`), `complete`, `stop`, `skipped`, `problems`, `coverage`, `per_skill` (each skill's `with`, `base`, `uplift`, `uplift_met` (`null` when not computed), its suites, `guards_failed` and `guards_watch`), `results` (each case's counts, and for a guard `guard_band` and `guard_held`), `non_discriminating`, `watch`, `graduates`, `granted_tools`, `scaffolded` and `libexec_target`. Findings: `eval/uplift/suite:<file>`, `uplift:<skill>`, `guard:<file>` (broken or failed confirmation), `watch:<file>`, `no-baseline`, `regressed:<skill>`, `non-discriminating:<file>:<grader>`, `granted-tools`, `scaffolded`, `libexec`, `read-only-sandbox`, `incomplete` and `cleanup`.

### Cost cap

`--max-cost-usd` defaults to 25 US dollars (wi-92bc1b, as Tier 2). The with arm gets at most half the cap, so the baseline always has the rest to compare against (with `--no-baseline`, which runs no baseline, the with arm gets the whole cap); each later batch gets what is left, rounded down to the cent, and none starts below a cent; the overrun is bounded by the runs in flight. A full run is 27 cases x 3 runs x 2 arms = 162 runs.

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
