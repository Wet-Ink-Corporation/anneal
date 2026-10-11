# Skill design rules and their checkers

`anneal lint skills [PATH]...` checks skill directories against the 21 skill design rules from
the blueprint. Each rule maps to at least one **checker**. A checker either decides mechanically
(an `error` or a `warning`) or, where no machine can judge the rule, lists it as a `review` item.
Nothing is passed silently. A test fails when a checker id is missing from this page.

```sh
anneal lint skills plugin/skills                       # what CI runs
anneal lint skills plugin/skills --index index.md      # also check the bootstrap index (rule 4)
anneal lint skills plugin/skills --cache .anneal/lint-skills.json   # skip unchanged skills
```

## Inputs

- Each `PATH` is a skill directory (it holds a `SKILL.md`) or a directory of skills. Files and
  subdirectories without a `SKILL.md` (a `README.md`, say) are ignored, so a directory with no
  skills is clean. The default `PATH` is `plugin/skills`. A path that does not exist, or that is
  a file rather than a directory, is a usage error (exit 2). So is a directory given to
  `--index`, `--hooks` or `--cache`.
- `--hooks FILE` is the `hooks.metadata.json` whose hook ids count as enforcers (rule 10). It
  defaults to `<first PATH>/../hooks/hooks.metadata.json` when that file exists. Only
  `hooks[].id` is read. If the file does not parse, that is an `enforcer` error.
- `--index FILE` is the session-start bootstrap index (rule 4). Without it, the index rule is
  listed as a review item.
- `--cache FILE` is the content-hash snapshot (below).

## Severity and exit codes

| Severity | Meaning | Fails the run |
|---|---|---|
| `error` | A rule is broken, decided mechanically | yes: exit 1 |
| `warning` | A rule is probably broken (a heuristic) | no, but listed |
| `review` | The rule needs judgment; a reviewer must look | no, but listed |

The exit code is 0 when there are no errors and 1 when there is at least one (see
`docs/exit-codes.md`). Warnings don't fail the run, because their checks are heuristics and a
false positive must not block a merge. They are still listed in every run.

## Output

The JSON envelope on stdout carries one `findings[]` entry per finding. Its `message` is one line:

```text
<file>:<line>: <severity> rule <n> [<checker>] <what is wrong> fix: <what to do>
```

`data` holds the counts (`checked`, `cached`, `errors`, `warnings`, `reviews`) and `details[]`,
the same findings as structured fields: `key`, `rule`, `checker`, `severity`, `file`, `line`,
`message`, `fix` and an optional `subject`. Line `0` means the path as a whole: a directory, or a
file with no single offending line. Keys (`lint/<checker>:<file>:<line>[:<subject>]`) are stable
across runs, so a baseline can tell new findings from old ones.

## Rules

| Rule | Summary | Checker | Severity | What is checked |
|---|---|---|---|---|
| 1 | Spec conformance | `frontmatter` | error | `SKILL.md` is UTF-8 and opens with a parseable frontmatter block (see Frontmatter) |
| 1 | | `name` | error | `name` is present, equals the directory name, is kebab-case (at most 64 characters) and does not start with `claude` or `anthropic` |
| 1 | | `description-budget` | error | `description` is present; `description` + `when_to_use` is 300 characters or fewer |
| 1 | | `listing-budget` | error | The model-invoked skills' `description` + `when_to_use` total 2,100 characters or fewer, reported at the skill whose running total crosses it (in path order) |
| 2 | Budget | `body-budget` | error | The body has 600 words or fewer and 500 lines or fewer; the whole `SKILL.md` is 8,192 bytes or fewer |
| 2 | | `references` | error | `references/` holds files only (one level deep), each is linked from the body (`references/<file>`), and every `references/...` link resolves |
| 2 | | `hard-rules-first` | warning | Every `MUST` line sits in the first 20 lines of the file |
| 3 | Anatomy | `anatomy` | error | `## Overview`, `## When to use`, `## Process`, `## Rationalizations`, `## Red flags`, `## Verification` all appear, in that order (other sections may sit between). Exempt skills: none (`ANATOMY_EXEMPT`) |
| 4 | Bootstrap | `bootstrap-index` | error / review | The `--index` file has 200 words or fewer; without `--index`, a review item |
| 4 | | `model-invocation` | error | Entry skills (below) are model-invoked; every other skill sets `disable-model-invocation: true`; the value is `true` or `false` |
| 5 | Descriptions | `description-trigger` | error | The description's first word is `Use`: the trigger, imperative, first |
| 5 | | `description-sibling` | error | The description contains `Do NOT use for ... (use `<sibling>`)`, every `(use `x`)` after the clause names another skill in the linted set, and there is at least one |
| 5 | | `overlap` | warning | Two skills' triggers (the description before `Do NOT use for`) share 50% or more of their content words (Jaccard; words of 3+ letters, stopwords removed). Feeds Tier 1 |
| 6 | Skip rationalization | `skip-rationalization` | warning | The description contains `even if` or `even when` |
| 7 | Form matches failure | `form-matches-failure` | review | Listed for every skill |
| 8 | Completion criteria | `completion-criteria` | warning | Every numbered step (`1.` or `1)` at the start of a line) under `## Process` contains "It's working if"; a Process with no numbered steps warns too |
| 9 | Thresholds are tables | `adjective-threshold` | warning | A prose line with a threshold adjective (`small`, `large`, `big`, `quick`, `few`, `many`, `tiny`, `huge`, `minor`, `trivial`) and a gate word (`gate`, `threshold`, `skip`, `block`, `require`, `allow`, `only`, `limit`, `cap`, `budget`, `must`, `unless` and their forms). Table rows, code blocks, `## Rationalizations` and `## Red flags` are skipped, since they quote the excuses on purpose |
| 10 | Name the enforcer | `enforcer` | warning | Every `MUST` line (outside code blocks) has a backticked enforcer on the same line: `` `anneal <command> ...` `` for a real top-level command, or a hook id from `hooks.metadata.json` |
| 11 | Facts looked up, decisions asked | `facts-and-decisions` | review | Listed for every skill |
| 12 | Scripts are run, not read | `scripts-contract` | review | Listed for a skill that ships a `scripts/` directory |
| 13 | Gates fail only on new findings | `gate-baseline` | review | Listed once per run; it governs gates, not skills |
| 14 | Hand subagents files | `subagent-handoff` | review | Listed for a skill whose prose mentions a subagent |
| 15 | Hook failure policy | `hook-failure-policy` | review | Listed once per run; it governs hooks |
| 16 | Stop-block cap | `stop-block-cap` | review | Listed once per run; it governs hooks |
| 17 | Cheapest layer decides | `check-order` | review | Listed once per run; it governs gates |
| 18 | Spec fields only | `frontmatter-fields` | error | Every frontmatter key is a spec field or a sanctioned extension (see Frontmatter) |
| 19 | Silent hooks | `hook-silence` | review | Listed once per run; it governs hooks |
| 20 | No `model` | `model-field` | error | No `model` key |
| 21 | Stable first, volatile last | `stable-first` | review | Listed once per run; it governs briefs and injections |

The once-per-run review items appear only when at least one skill was linted. They point at the
first `PATH`, line 0.

## Entry skills

The model-invoked entry skills are a constant in the linter (`ENTRY_SKILLS` in
`crates/anneal-cli/src/lint/skills/checker.rs`), taken from the blueprint's packaging decision:

`grill`, `spec`, `plan`, `build`, `diagnose`, `verify`, `ship`

Every other skill (the user-invoked `orchestrate`, `handoff`, `correct`, `write-skill` and
`calibrate`, and any new one) must set `disable-model-invocation: true`. The list lives in the
linter, not in `hooks.metadata.json`, because that file describes hooks and its schema belongs to
#13. Changing the list is a reviewed code change, like any other exemption.

## Frontmatter

There is no YAML library. The parser reads this subset and reports anything else as a
`frontmatter` error with the line it stopped at:

- `---` on the first line, and a closing `---` line
- `key: value`, with a plain, `"double"` (escapes `\"` and `\\`) or `'single'` (escape `''`)
  quoted scalar; `# comment` lines, and a trailing ` # comment` after a value. A plain
  (unquoted) value may not contain `: `, end with `:`, or start with `` ` ``, `@` or `%`,
  because strict YAML rejects those: quote it instead
  (`description: "Use when: ..."`)
- `key: >` (folded) and `key: |` (literal) blocks of indented lines; chomping indicators
  (`>-`, `|+`, ...) are accepted, and trailing newlines are always stripped
- `key:` followed by an indented block (a nested map such as `metadata:`), kept as raw text
- no duplicate keys, no unexpected indentation

Allowed keys:

- Agent Skills spec: `name`, `description`, `license`, `compatibility`, `metadata`,
  `allowed-tools`
- Sanctioned Claude Code extensions that the rules call for: `when_to_use` (rule 1),
  `disable-model-invocation` (rule 4), `context` and `agent` (rule 20's fork), and
  `argument-hint` (user-invoked skills)

`model` is rejected by its own checker (rule 20). Any other key is a `frontmatter-fields` error.
Put it under `metadata:` instead.

## Content-hash snapshot

With `--cache FILE`, each skill's per-skill findings are stored under a SHA-256 of its
directory, plus the source code of the checks, the anneal version and the hook ids. Every input
is length-prefixed. Within the directory, each entry contributes its relative path and kind,
then:

- a regular file: its size and its first 1 MiB
- a symlink: its target path, never followed (so a dangling link or a link to `/dev/zero` is
  safe)
- a directory: its own entries, down to four levels
- a FIFO, socket or device: nothing more

Because the checks' own source is hashed, editing a check invalidates every entry; no version
number has to be bumped. A skill whose hash is unchanged is not checked again; its stored
findings are replayed, so a cached run reports exactly what a fresh run would. Set-level checks
(`listing-budget`, `description-sibling`, `overlap`, `bootstrap-index` and the once-per-run
reviews) always run. A JSON object with a numeric `version` that is another version, or
another shape, is stale: it is ignored and rebuilt. Any other file at that path is refused with
exit 70 rather than overwritten. `data.cached` counts the replayed skills.
