# The anneal mod (cockpit)

anneal ships a [Claude Code mod](https://code.claude.com/docs/en/plugins/mods/overview) inside the plugin: a TypeScript hooks module that Claude Code runs in its own process. The rule it follows is #56's: **hooks enforce, the mod shows and collects.**

- Every gate stays a settings hook in `plugin/hooks/hooks.json` calling the `anneal` CLI. The mod gates nothing.
- The mod holds no logic of its own, and no state but its last reading of the CLI (kept in `$.state` so a drawing survives a hot reload). It renders what `anneal status --json` prints, through the plugin's launcher (`$.plugin.root + "/libexec/anneal"`), and writes nothing to the ledger.
- anneal works fully with no mod: where mods are off or not supported, every gate still runs.

## What it does (#57, #59, #63)

The cockpit reads `anneal status --json` (`docs/status.md`) and draws it in two places, the band and the pane. Asked with `/anneal calibration`, it also reads `anneal calibrate` (`docs/calibrate.md`) and draws the calibration dashboard. It writes nothing to the ledger and runs no other `anneal` subcommand.

### The band

A `ui.render` hook on `AbovePrompt` draws one row, cut to `e.props.bodyColumns`:

```text
anneal · build · plan ✓ a1b2c3 · task 3/7 · ctx 58%
anneal · build · plan ✓ a1b2c3 · task 3/7 · 1 gate open · ctx 58% · guard.rm: rm -rf outside the worktree
```

- The mode (`phase.mode`); the plan (`plan ✓ <first six hex digits of the approved hash>`, `plan stale`, or `plan unapproved`); `task <done>/<total>`; `<n> gate(s) open`; `ctx <percent>%` (the newest context reading). A section the document leaves out is left out of the row.
- Dim while nothing blocks. While a gate is open the row is drawn in the theme's `warning` color and ends with the newest open gate's id and reason.
- Nothing at all while there is no ledger (`anneal status` exits 2 with `status/no-ledger`), before the first refresh has read anything, and while a survey holds the band: the hook passes to the engine (`next(e)`).
- `anneal · status unavailable` when the CLI cannot be run or prints no status document and there is no earlier good reading. It is drawn, not logged, and the next refresh tries again.

**Open gates.** `anneal status` lists the gates that recently *blocked* (`gate_blocked`, `stop_blocked`), not the ones still open (`docs/status.md`, `gates`). The cockpit calls a block *open* when nothing but other blocks has been recorded since it: its ledger position is after the newest event of `ledger_tail` that is not a block. A gate the session got past (a later `task_done`, `test_run`, approval, ...) is shown as passed.

### The pane

`/anneal` opens a pane with `$.ui.open({ id: "anneal" })`. The surface places it: docked beside the transcript in the fullscreen terminal (the command asks for 56 columns there, as `presentation.isFullscreen` says), inline above the prompt otherwise (it asks for 18 rows). Its sections:

| Section | From | Shows |
|---|---|---|
| Gates | `gates` | newest first, each a `Button` (`▸ #<event> <gate> open` or `passed`) that expands its event: kind, time and reason; a second press folds it |
| Plan | `plan` | the plan's path, `approved`/`stale`/`none`, its hash and when it was approved |
| Tasks | `tasks` | done of all, and the next task |
| Verdict for HEAD | `verdict` | the outcome, the pull request, the fresh verdict and why it is fresh |
| Clef | `clef` | per point: the mode in effect, answered decisions, and the configured mode where shadow holds it back |
| Ledger | `ledger_tail` | the newest 10 events, oldest first |

**The pane never opens unasked**: only `/anneal` opens it, and the person closes it (its close mark, ctrl+x x). Which gate is expanded is kept in `$.state` too, so it survives a hot reload.

### Refresh

The cockpit asks for a refresh at `session.start`, at `turn.complete`, and after a `Bash`, `Edit` or `Write` call has settled (the hook awaits `next(e)` and returns its result unchanged; it never rewrites or refuses a call, and its `.catch` hands back the call's settled result if the hook fails). A refresh:

- runs `anneal status --json --fields phase,plan,tasks,gates,context,ledger_tail --tail 10` through the launcher with `$.process.run` and a 2 s timeout, adding `verdict,clef` while the pane is open (the verdict section is the one that runs subprocesses);
- starts at most once a second: an ask within a second of the last start waits for that second to pass, and every ask meanwhile folds into it (ten tool calls in one second are at most two runs);
- puts the result in `$.state` (`anneal.reading`), writing only when it changed, so a hot reload keeps it and a write redraws only the band and pane that read it;
- keeps the last good reading when the CLI times out or fails, and logs nothing to the transcript (a refusal by the host of a call the refresh needs goes to the debug log only).

The pacing is the module's own and starts over on a reload; the reading does not.

### Where nothing draws

Where no surface is attached (a cloud session, Claude Code for VS Code today, `claude -p`), or the surface does not place the pane, `/anneal` closes the pane it asked for and answers `{ text }` with `anneal status`'s text form (with or without a ledger), given 10 seconds. When the CLI cannot give one it answers one line: `anneal status is not built yet (<the first line the CLI wrote, or why no launcher started>).`

### Launchers

Each run uses the sh launcher `libexec/anneal`, and `libexec/anneal.cmd` only when the first cannot start (Windows). Once one has started the cockpit keeps to it, so a run that times out is not tried again through the other.

### Calibration (#63)

`/anneal calibration` shows how close each Clef decision point is to promotion. It runs `anneal calibrate` (there is no `--json` flag: stdout is always the envelope, `docs/calibrate.md`, "The `data` schema") through the launcher with a 10 s timeout and keeps the result in `$.state` (`anneal.calibration`).

- **Where.** On a terminal of 100 columns or more it opens a second pane, `anneal-calibration` (title `calibration`), which the surface shows as a tab beside `anneal`. Below 100 columns it is a Calibration section at the end of the `anneal` pane, which it opens; `/anneal` alone takes the section away again. Where nothing draws, the command answers with the same rows as text.
- **One row per point, D1 to D7**, each a `Button`, in two lines so it fits a docked pane: `▸ D1 shadow █░░░░░░░░░ 5/50 · short of labels`, then `Brier 0.2885 · ask 33% · threshold 0.4 → 0.3`. The mode comes from `anneal status`'s `clef` section (`unknown` until a status reading has it; the refresh asks for `clef` while the calibration pane is open), the rest from the point's first question with bands (D3's `choice`, not its bandless `score`). The bar is labelled decisions out of 50. Readiness is `ready` (calibrate's `eligible-to-enforce`), else `short of labels`, `Brier too high` or `ask rate too high`, from calibrate's reasons in that order. Nothing is recomputed in the mod.
- **Selecting a row** expands each of the point's questions: n, labelled, unanswered and asks; the threshold now and as proposed with both expected costs (or why there is no proposal); the reliability diagram; and a line per bin with decisions (range, count, mean predicted, observed). The diagram is a `Raster` on the terminal (the only surface with it) and an `Svg` on the desktop and the other remote surfaces, chosen by narrowing `e.surface`: observed against predicted, the diagonal dotted, each bin with decisions a dot, hollow (`○`, an unfilled circle) when it holds fewer than 5 decisions. On the raster each bin owns two columns, so bins never overlap, and observed is drawn in tenths. A test reads both trees back and checks they agree on every bin.
- **No apply button.** The pane ends with "Proposals only, nothing applied. A person promotes with `anneal calibrate --apply --thresholds <file>`" (#44). `/anneal calibration` takes only `refresh`; any other argument prints its usage.
- **Refresh** on open and on `/anneal calibration refresh`, never on a timer, a turn or a tool call. A run under way absorbs another ask. A failed run after a good one keeps the good one on screen with `Last refresh failed: <reason>`; with none, the pane says `anneal calibrate unavailable: <reason>`. Nothing is logged to the transcript.

`anneal calibrate` is read-only toward the thresholds and the mode, but each run rewrites `<anneal home>/decisions/proposals.json` (`docs/calibrate.md`). Opening or refreshing the dashboard therefore replaces that file, as running the command by hand does.

**Not in the envelope (follow-ups for Ryan).** `anneal calibrate` carries no point's Clef mode, so the mod joins `anneal status`'s `clef` section by point name. It gives no reliability bins beyond the ten equal-width ones and no history of earlier runs, so the dashboard shows the latest run only, not how the numbers moved.

### Screenshots (manual, for Ryan)

Screenshots of the band (dim and blocked), of the pane (docked and inline) and of the calibration tab (terminal raster and desktop SVG) were not taken: drawing them needs an interactive session, which calls a model, and this story ran none. Take them with `claude --plugin-dir plugin` in a repository with a ledger and add them here.

## Files

| Path | Ships | What it is |
|---|---|---|
| `plugin/.claude-plugin/plugin.json` | yes | `"hooks": "./hooks/cockpit.json"` (merged with `hooks/hooks.json`) and `"types": "./types/index.d.ts"` |
| `plugin/hooks/cockpit.json` | yes | `{ "modules": ["./cockpit.tsx"] }` |
| `plugin/hooks/cockpit.tsx` | yes | the hooks module |
| `plugin/types/index.d.ts` | yes | the status document's and `anneal calibrate`'s types and the `PluginState` contract: `anneal.reading` (the last reading: unread, absent, read or unavailable), `anneal.expanded` (the pane's expanded gate), `anneal.calibration` (the last calibrate run), `anneal.selected_point` (the point whose diagram is shown) and `anneal.calibration_section` (drawn inside the `anneal` pane on a narrow terminal) |
| `plugin/tests/cockpit.test.ts` | yes | the band and pane on `terminal` and `desktop` from fixture status JSON (no ledger, mid-build, a blocked gate), the refresh's rate limit and timeout, and `/anneal` where nothing draws; the calibration dashboard on both surfaces from fixture calibrate JSON (no data, partial, one point ready), the raster and SVG read back and compared bin by bin, its refresh, failure, narrow section and text form; run by `claude plugin test plugin` |
| `plugin/tsconfig.json` | yes | strict, `noUncheckedIndexedAccess`, the options Claude Code's declaration header gives for a hooks module |
| `mod-types/mod-api.d.ts` | no | a hand-written declaration of the few mods API types the module and test use (see below) |

There is no packaging exclude list (the release zips `plugin/` whole and there is no `scripts/package-plugin.sh`), so the test and `tsconfig.json` ship. They are small and inert: Claude Code loads only the module `cockpit.json` names, and `claude plugin test` is the only thing that reads `*.test.ts`. The plugin still has no `bin/` and no `evals/`.

`mod-types/mod-api.d.ts` is our own minimal declaration, in our own words, of only the API surface `cockpit.tsx` and its test use, mirroring the [mods reference](https://code.claude.com/docs/en/plugins/mods/reference) as of Claude Code 2.1.303. Claude Code's own declaration file is not vendored: it is not under this repository's licence. Nor can CI point `tsc` at it: the npm package `@anthropic-ai/claude-code` (2.1.303, and the 2.1.42 installed here) ships only `sdk-tools.d.ts`, and Claude Code writes its mods declarations only when an interactive session hot-reloads a mod. So `tsc -p plugin` checks the code against the shim, and the real engine checks it in CI through `claude plugin validate --strict` and `claude plugin test` on the pinned version. On 2026-10-19 the module, contract and test also type-checked against Claude Code 2.1.303's own declarations, in a throwaway config outside the repository. Extend the shim only as the mod uses more of the API.

## Checks

```sh
tsc -p plugin                       # type-check the module, contract and test
claude plugin validate --strict plugin
claude plugin test plugin           # no model is called: the test answers $.process.run itself
```

`claude plugin test` runs each `*.test.ts` in an environment like the module's (no fs, network or process); the test's own hooks stand for the host: `process.run` (the CLI, answered from fixture `anneal status --json` envelopes), a mocked clock, `session.surfaces`, `ui.open`, `ui.panes`, `ui.close` and `ui.log`. The band and pane are mounted on `terminal` and `desktop` in one loop. On 2026-10-19 the module, contract and test (#59) also type-checked against Claude Code 2.1.303's own declarations, in a throwaway config outside the repository.

## Turning it off

Claude Code has no switch that stops anneal's mod and keeps anneal's settings hooks; the gates are unaffected by the mod either way. What exists today (Claude Code 2.1.303):

| Switch | Mod | anneal's settings hooks | Skills |
|---|---|---|---|
| Disable or uninstall the plugin in `/plugin` | off | off | off |
| `"disableAllHooks": true` in `~/.claude/settings.json` | off | off | on |
| `claude --safe-mode` (one session) | off | off, per `claude --help` | off |
| Organization `allowManagedModsOnly` (managed settings) | off | on, per the docs ("users' settings hooks keep running"; not checked for a plugin's) | on |

So `--safe-mode` is not a way to keep the gates while dropping the mod: its help says it starts "with all customizations (CLAUDE.md, skills, installed plugins, hooks, ...) disabled". A mod-only off switch would be a `userConfig` option the module reads in `register` (a decision for Ryan, not built).

## Packaging spike: evidence per host

The question was whether one `plugin/hooks/hooks.json` can hold both the settings `hooks` and the mod's `modules`.

| Host | Result | How it was checked |
|---|---|---|
| Claude Code CLI 2.1.303, `claude plugin validate --strict` | **passes** with both keys in `hooks/hooks.json` (it lists the module's hooks and calls), and passes with the layout shipped (`modules` in `hooks/cockpit.json`, named by `plugin.json`'s `hooks`). With the manifest key set, validate still reads `hooks/hooks.json` too: an unknown hook type planted there failed `--strict`. | A scratch copy of `plugin/` under `target/`, edited and validated, 2026-10-19 |
| Claude Code CLI 2.1.303, `claude plugin test` | **loads** the module from the shipped layout: four tests pass, and one fails when `statusText` is broken (#57); twelve pass with the band and pane (#59), and the rate-limit test fails when the one-second interval is set to 0; twenty-three pass with the calibration dashboard (#63), and the raster-and-SVG test fails when the SVG's dots are moved up a tenth | `claude plugin test plugin` |
| Claude Code CLI, a live session (settings hooks and the mod both running) | **unverified**: starting a session calls a model, which this story did not do | manual check below |
| Claude Desktop (Code tab), VS Code extension | **unverified**: docs say mods run there (Desktop from 2.1.286) | manual check below |
| claude.ai plugin upload, Cowork plugin upload | **unverified, for Ryan**: cannot be checked from a CLI session. The zip still has no `bin/` (the documented reason claude.ai and Cowork refuse a plugin); whether either accepts a `hooks.json`-referenced file holding `modules`, a `types` manifest key, `.tsx`/`.ts` files or a `tsconfig.json` is not known. Hooks and mods do not run there anyway, so the question is only whether the upload is accepted and the skills load | upload `anneal-plugin-<version>.zip` from a release built after this change |
| Cloud sessions | docs: hooks run for a plugin that reaches the session, nothing is drawn | not checked |

The docs' own layout is the single file ("`hooks/hooks.json`: `modules` ... Can also hold settings hooks under `hooks`", [mods reference](https://code.claude.com/docs/en/plugins/mods/reference#files)). The shipped layout uses a second file only because `crates/anneal-cli/tests/it/hooks_manifest.rs` reads `hooks.json` with `deny_unknown_fields` (`HooksFile` has only `hooks`), and this story changes no Rust. Moving `modules` into `hooks.json` needs that DTO to accept `modules` (an H-12/H-16 test change for Ryan); `plugin.json`'s `hooks` key then goes.

### Manual checks

Run these in a terminal session with the plugin loaded (`claude --plugin-dir plugin`, or installed from the marketplace):

1. `/plugin` shows `1 mod active · anneal` under its tabs.
2. The band shows `anneal · <mode> · ...` above the prompt in a repository with a ledger, and nothing in one without. `/anneal` opens the pane (docked in fullscreen, inline otherwise), each gate row expands on Enter or a click, and the band follows a Bash, Edit or Write call within about a second. In `claude -p` or the VS Code panel, `/anneal` prints `anneal status`'s text form.
3. A settings hook still runs beside the mod: `rm -rf /` through the Bash tool is denied by the guard.
4. `claude --safe-mode`: `/anneal` is unknown and `/plugin` lists no mod. Note whether the guard still denies `rm -rf /` (the help says hooks are off in safe mode; #57 expected them to keep running).

## CI job (to install)

`.github/` is protected from agent edits, so the job is here for Ryan to paste into `.github/workflows/ci.yml` under `jobs:`. Checkout is pinned to the SHA the existing jobs use. Node is the runner's preinstalled one, as the existing `plugin` job's `npm install -g` relies on; no `actions/setup-node` pin exists in this repository to copy, so add one only with a SHA you choose. Claude Code is pinned to 2.1.303 (the version `mod-types/mod-api.d.ts` mirrors; at least the 2.1.295 the README requires) and TypeScript to 6.0.2.

```yaml
  mod:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - name: Claude Code and TypeScript (pinned)
        run: npm install -g @anthropic-ai/claude-code@2.1.303 typescript@6.0.2
      - name: type-check the mod
        run: tsc -p plugin
      - name: claude plugin validate --strict
        run: claude plugin validate --strict plugin
      - name: claude plugin test (no model is called)
        run: claude plugin test plugin
```
