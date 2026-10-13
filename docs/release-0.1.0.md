# anneal 0.1.0 release notes (draft)

Draft for story #44, the Release 6 exit. The release notes the workflow publishes are the
`## 0.1.0` section of `CHANGELOG.md` (`scripts/changelog-section.sh`); this page holds what goes
with them: the per-point Clef verdicts, the release checklist and the acceptance checks.

## Clef verdicts

Every Clef decision point ships in shadow unless its calibration earns enforcement. Ryan runs
`anneal calibrate` against his dogfood ledger (the decision log in his anneal home) and promotes
a point only when it meets the bar from `docs/calibrate.md`:

- **n ≥ 50** labelled decisions (labelled with `anneal decide label`, not merely answered),
- **Brier ≤ 0.15**,
- **ask rate < 20%**.

A point that meets the bar is promoted by a human setting its mode (`clef_mode`, for example
`shadow,D1=enforce`); `anneal calibrate` never changes a mode. Every other point stays in shadow,
and its row says why, for example "stays in shadow (n = 23)".

| Point | What it decides | n | Brier | Ask rate | Verdict |
|---|---|---|---|---|---|
| D1 guard | Is a Bash command the denylist did not decide destructive or irreversible? (`noul destructive_or_irreversible`: deny at 0.90, ask at 0.40) | to fill from `anneal calibrate` | to fill from `anneal calibrate` | to fill from `anneal calibrate` | stays in shadow |
| D2 orchestrate | Is a judgment call during `/anneal:orchestrate` a one-way door, to park in weigh-in rather than rule on? (`noul one_way_door`: park at 0.50) | to fill from `anneal calibrate` | to fill from `anneal calibrate` | to fill from `anneal calibrate` | stays in shadow |
| D3 route | Which phase does a prompt belong to, for the router hint? (`choice phase`: hint at 0.60; `score size` is recorded only and does not count) | to fill from `anneal calibrate` | to fill from `anneal calibrate` | to fill from `anneal calibrate` | stays in shadow |
| D4 completeness | Is each required section of the final report present and grounded in the ledger? (`noul present_and_grounded` on `clef`: pass at 0.50, else block) | to fill from `anneal calibrate` | to fill from `anneal calibrate` | to fill from `anneal calibrate` | stays in shadow |
| D5 verify | How risky is the change, and may the doubt review be skipped? (`score defect_risk`: doubt review at 0.6666, else skip) | to fill from `anneal calibrate` | to fill from `anneal calibrate` | to fill from `anneal calibrate` | stays in shadow |
| D6 ruled-before | Has this prompt's question been ruled on already, and which ruling? (`choice ruled_before`: surface at 0.70) | to fill from `anneal calibrate` | to fill from `anneal calibrate` | to fill from `anneal calibrate` | stays in shadow |
| D7 grill | Is a grilling question a decision for the human rather than a fact to look up? (`noul is_decision`: ask the human at 0.50, else find the fact) | to fill from `anneal calibrate` | to fill from `anneal calibrate` | to fill from `anneal calibrate` | stays in shadow |

The bands are the defaults in `plugin/playbooks/clef-thresholds.json` (`docs/decide.md`). Only
D1 (`ask`) and D7 (`ask_human`) hand a decision to a human, so the other points' ask rate is 0.
Fill n, Brier and ask rate from the stderr summary or `data.points` of `anneal calibrate`; for
D3, report the `choice` question. If calibration proposes a threshold, it is applied only with
`anneal calibrate --apply --thresholds <file>` after Ryan approves the diff, and the new value
belongs in this table's row.

## Release checklist

- [ ] Version bumped to `0.1.0` in `plugin/.claude-plugin/plugin.json` and `Cargo.toml`,
      together (`scripts/check-versions.sh` fails when they differ).
- [ ] `CHANGELOG.md`: `## Unreleased` moved to `## 0.1.0 — <date>`. The release workflow refuses a
      tag whose version has no section, and publishes that section as the release notes.
- [ ] README: install from the marketplace, minimum Claude Code 2.1.295, the one-time status line
      setup, Clef `userConfig`, and the harness scope (skills portable; hooks and Clef in Claude
      Code only).
- [ ] `CREDITS.md` complete for all re-authored techniques, with each "not yet confirmed" licence
      and missing repository link settled.
- [ ] Tier 0 to 3 suites green on `main` (`evals/README.md`). Tiers 0 and 1, and the Tier 3
      suite check, run in CI through `cargo test`. Tiers 2 and 3 have no CI job yet, so Ryan
      runs them locally, in a container, with `claude` signed in; they spend money:
      `anneal eval routing --router claude --trust-plugin` and
      `anneal eval uplift --trust-plugin`.
- [ ] The 7 model-invoked descriptions total 2,100 characters or fewer (`anneal lint skills`).
- [ ] The Clef verdict table above is filled in, and any promoted point's mode is recorded.
- [ ] `v0.1.0` tagged on `main` and pushed, by Ryan.
- [ ] The release workflow publishes the GitHub release with the zip, the binaries and
      `SHA256SUMS`.
- [ ] The `marketplace` branch is refreshed for Claude Code installs (see below: the workflow
      does not do this yet).

### What `.github/workflows/release.yml` does today

On a pushed tag `v*` it:

1. checks that the tag's version equals the version in `plugin.json` and `Cargo.toml`, that the
   tagged commit is on `origin/main`, and that `CHANGELOG.md` has a non-empty section for it;
2. runs `ci.yml` (format, clippy with `-D warnings` and tests on Linux, macOS and Windows; the
   skill lint; the launcher smoke tests; the version check; no `bin/` under `plugin/`; and
   `claude plugin validate --strict` on the marketplace and the plugin);
3. builds release binaries for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
   `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`;
4. places them in `plugin/libexec/<target>/` (plus a copy of the Windows build at
   `plugin/libexec/anneal.exe`), zips `plugin/` as `anneal-plugin-<version>.zip`, copies each
   binary as `anneal-<target>` (`.exe` on Windows) and writes `SHA256SUMS` over all of them;
5. creates the GitHub release `anneal v<version>` with those assets and the changelog section as
   its notes, marked pre-release only for `v0.0.*` (a re-run uploads the assets again with
   `--clobber` and rewrites the notes).

It does **not**:

- refresh a `marketplace` branch. Nothing in the repository creates or updates one, so a
  marketplace install reads `main`, whose `plugin/libexec/` holds only the launchers: the
  per-target builds are not committed (`.gitignore`). Until the branch exists, an install from
  the marketplace has no binary, and its guards block Bash, Edit, Write and Agent calls unless an
  `anneal` is on PATH. Adding the refresh is a change to `.github/workflows/`, a maintainer's
  edit (issue #44 points at weigh-in D-0009).
- call `scripts/package-plugin.sh`, which the issue names; that script does not exist, and the
  zip is made inline in the workflow.
- run `rust.yml` (temper's full gate on Linux); it runs on pushes to `main`, so the tagged commit
  has already passed it there.
- run Tier 2 or Tier 3.
- build `x86_64-apple-darwin`, which the launcher would pick on an Intel Mac; there it falls back
  to an `anneal` on PATH.
- sync an organization repository for claude.ai and Cowork (weigh-in B-19); its name is to be
  confirmed with Ryan first.

## Acceptance checks

- [ ] The `v0.1.0` GitHub release exists with `anneal-plugin-0.1.0.zip`, the four binaries and
      `SHA256SUMS`, and `sha256sum -c SHA256SUMS` passes over the downloaded assets.
- [ ] On a clean machine with no Rust toolchain and no `anneal` on PATH, installing from the
      marketplace (`/plugin marketplace add Wet-Ink-Corporation/anneal`, then
      `/plugin install anneal@anneal`) works, the session start shows no "anneal is not
      installed" hint, and the hooks fire (for example the guard denies a destructive Bash
      command).
- [ ] Uploading `anneal-plugin-0.1.0.zip` to claude.ai is accepted: the zip has no `bin/`
      (`unzip -l anneal-plugin-0.1.0.zip | grep -E '(^|/)bin/'` prints nothing).
- [ ] These release notes, with the Clef verdict table filled in, are published with the release.
