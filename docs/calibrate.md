# `anneal calibrate`

`anneal calibrate [--point <D1..D7>] [--thresholds <file>]` checks each Clef decision point
against the outcomes labelled with `anneal decide label`. For each point it reports the Brier
score, reliability bins, the ask rate, a proposed threshold and a promotion verdict. It writes
the proposal to the anneal home and prints the manual steps that would apply it. It never
applies anything itself: no point moves to `enforce` and no threshold changes until a human
makes the edit.

## Input

The decision log `anneal decide` writes (`docs/decide.md`):
`<anneal home>/decisions/decisions.4.jsonl` down to `decisions.1.jsonl`, then
`decisions.jsonl`, read oldest first. The anneal home is `$ANNEAL_HOME`, else `~/.anneal`, as
`decide` resolves it. Each file must be a regular file; a missing file is skipped.

`decide` rotates the live file at 16 MiB, but it appends without a lock. Concurrent calls can
each pass its size check and leave the live file a few rows past 16 MiB. So `calibrate` reads
a file of up to 16 MiB plus 64 rows of decide's 8 KiB row cap (16.5 MiB in all). Nothing
`decide` writes is larger than that, so a larger file is an error (exit 70), never cut short.

Rows are read tolerantly, so fields a newer `decide` adds are ignored. Lines are handled as
follows:

- A line that is not a decision or label row, or a probability outside 0..=1, is counted in
  `data.skipped_rows`. One torn line does not stop calibration. A probability of `-0.0` is
  read as `0.0`, so it sorts and bins with `0.0`.
- A decision with a probability is **answered**; one without (unreachable, timed out, bad
  response) is counted as `unanswered`.
- A label row attaches to the decision with its `id`. When an id has several labels, the last
  one read wins.
- Labels whose id is no answered decision in the log are counted in `data.unmatched_labels`.
  That covers an unanswered decision, one rotated out of the log, or a mistyped id.
- Rows of a question the thresholds no longer define are counted in `data.unreported_rows`.

**Labels come only from `anneal decide label`.** The blueprint's automatic sources (for D1, the
human approving or denying when asked, or a revert within 24 h) need hooks that do not exist
yet. They are a follow-up. When they land, they write label rows just as `decide label` does.

### What a label means

| Question | Positive (o = 1) | Negative (o = 0) | Anything else |
|---|---|---|---|
| noul, score | `yes`, `true`, `1` | `no`, `false`, `0` | unusable |
| choice | the decision's top option | another option the row lists | unusable |

The predicted probability p is the one the thresholds applied to: P(`yes`) for noul, the top
option's probability for choice, and the value itself for score. An unusable label is counted in
`unusable_labels` and does not count as labelled.

## Per point and question

Each point lists every question its thresholds define (D3 has `choice` and `score`):

- `n`: answered decisions. `labelled`: answered decisions with a usable label. `unanswered`
  and `unusable_labels` are counted separately.
- `brier`: the mean of (p - o)^2 over the labelled decisions (`null` with none).
- `bins`: 10 equal-width bins [k/10, (k+1)/10), the last closed at 1.0. Each bin gives `lower`,
  `upper`, `count`, `mean_probability` and `observed_frequency` (the fraction of positives).
  An empty bin has `null` for the last two.
- `asks` and `ask_rate`: answered decisions whose action handed the decision to a human, out of
  `n`. An ask is an action named `ask` or starting `ask_`: D1's `ask` and D7's `ask_human`.
  Points without such an action have an ask rate of 0.
- `threshold`: the point's lowest band, which is the boundary between acting and `otherwise`.
  It gives `band` (its action), `pointer` (its `at_least` in the thresholds file),
  `current`, `ceiling` (the next band's `at_least`), `false_accept`, `current_expected_cost`,
  `proposed`, `proposed_expected_cost` and `no_proposal` (why there is none).
- `verdict` and `reasons`.

A point's `verdict` is `eligible-to-enforce` only when it has decisions on a question whose
rule has bands and every such question with decisions is eligible. A question with no
decisions does not hold the point back, and neither does a bandless question, such as D3's
size score: it only records and never acts, so its own verdict is reported but not counted
(the same questions `decide` counts toward the 50-decision shadow period).

### Verdict

A question stays in shadow (`stay-shadow`) for any of these reasons:

| Reason | Rule |
|---|---|
| `too_few_labelled` | fewer than 50 **labelled** decisions |
| `brier_above_bound` | Brier score over 0.15 |
| `ask_rate_at_or_above_bound` | ask rate of 20% or more |

Otherwise it is `eligible-to-enforce`. The 50 counts labelled decisions, not answered ones,
so a point with 49 labelled decisions is never eligible, however good they are.

### Expected cost and the proposed threshold

A threshold t splits the labelled decisions: at or above t the point acts (asks, denies,
parks, passes, hints), and below it the point takes `otherwise`. There are two kinds of error:

- A **false accept** costs **5**: the error the point exists to prevent gets through.
- A **needless intervention** costs **1**: an ask, block or hint that was not needed.

Which side is which depends on the point:

| Point | The point acts on | False accept (5) | Needless (1) |
|---|---|---|---|
| D1 guard, D2 orchestrate, D5 verify, D7 grill | p >= t | a positive below t: let through | a negative at or above t: asked or reviewed for nothing |
| D3 route, D4 completeness, D6 ruled-before | p >= t | a negative at or above t: a wrong hint, an incomplete section passed, the wrong ruling surfaced | a positive below t: help withheld, a stop blocked for nothing |

The expected cost of t is its total cost divided by the labelled count. The candidates are:

- the current threshold;
- each distinct labelled probability strictly below the next band up, so the bands stay
  strictly descending (for D1 that means below deny's 0.9);
- one threshold above every labelled probability below the next band: halfway between the
  highest of them and the next band (or 1.0 for the top band), so it acts on none of them.

The cheapest of the new candidates is proposed only when its total cost is strictly less than
the current threshold's. A tie between new candidates goes to the higher threshold, which acts
on fewer decisions. Otherwise `proposed` is the current value, and there is no apply step: a
proposal never costs more than leaving the threshold alone. There is no proposal (`proposed`
is `null`) for a rule without bands (`no_bands`, D3's size score) or with no labelled
decisions (`no_labels`).

The costs are constants. The thresholds file is read strictly (`docs/decide.md`), so per-point
costs there would change its schema. That is a follow-up if the 5:1 ratio turns out wrong for
a point.

### A worked example

`crates/anneal-cli/tests/fixtures/calibrate/` holds a log worked through by hand. For D1, the
labelled decisions (p, o) are (0.1, 0), (0.3, 1), (0.8, 1), (0.95, 0) and (1.0, 1):

- **Brier.** The squared errors are 0.01, 0.49, 0.04, 0.9025 and 0. They sum to 1.4425, so
  the Brier score is 0.2885.
- **Bins.** [0.1, 0.2) holds 1 decision, observed frequency 0. [0.3, 0.4) holds 1, frequency 1.
  [0.8, 0.9) holds 1, frequency 1. [0.9, 1.0] holds 2, mean probability 0.975, frequency 0.5.
- **Current cost.** At the current 0.4, the decision at 0.3 is a false accept (5) and the one
  at 0.95 is a needless ask (1): 6 / 5 = 1.2.
- **Proposal.** The candidates cost 6 (the current 0.4), 2 (0.1), 1 (0.3), 6 (0.8) and 11
  (0.85, halfway from 0.8 to 0.9). The proposal is 0.3, at 1 / 5 = 0.2, strictly cheaper
  than the current threshold.

## Output

stdout carries the envelope (`docs/exit-codes.md` conventions, exit 0). Its `data` holds:

- `log` (the live file), `files` (the files read), `proposals` (the proposals file) and
  `thresholds` (the file read, or `null` for the copy built in).
- `costs` (`{false_accept: 5, needless: 1}`), `skipped_rows`, `unmatched_labels` and
  `unreported_rows`.
- `points` and `apply`.

With `--point`, `points` and `apply` hold that point only.

stderr carries a short summary, written after the envelope. It has a line per point with its
verdict, a line per question with n, labelled, Brier, ask rate, the threshold now and as
proposed (or "kept (nothing cheaper)"), and the reasons. Then it gives the proposals file and
each manual step. Errors are one `ERROR` line as for every command (exit 70 for a log,
thresholds or proposals file that cannot be read or written, or a log file over 16.5 MiB).

The summary is best effort. It is written and flushed only after the envelope, which already
says `ok: true` and holds everything the summary repeats. If stderr refuses the summary or its
flush, the run still exits 0. Nothing else is written then: stderr is the stream that failed,
and stdout already holds its one envelope.

### The proposal and how to apply it

Every run overwrites `<anneal home>/decisions/proposals.json` atomically. The file holds
`{kind: "anneal-threshold-proposals", version: 1, thresholds, costs, points, apply}` with
**every** point, whatever `--point` selected.

There is no apply command. `apply` lists the manual steps (a follow-up may add an
`anneal calibrate apply`):

- `{"kind": "threshold", "point", "question", "file", "pointer", "from", "to"}`: set the JSON
  pointer's value in the thresholds file. When `file` is `null`, the copy built into the
  binary was read. Copy `plugin/playbooks/clef-thresholds.json`, edit the copy, and pass it
  with `--thresholds` or install it as `$CLAUDE_PLUGIN_ROOT/playbooks/clef-thresholds.json`.
  A step appears whenever the proposal differs from the current value, verdict or not.
- `{"kind": "mode", "point", "user_config": "clef_mode", "env":
  "CLAUDE_PLUGIN_OPTION_CLEF_MODE", "value": "Dn=enforce"}`: add the override to the plugin's
  `clef_mode` setting, for an `eligible-to-enforce` point only.

`decide` keeps its own guard: a point set to `enforce` still runs in shadow until it has 50
answered decisions (`docs/decide.md`).

## Limits

- A rotation during the read can skip a file or read one twice. Run it again.
- The ask rate counts the actions that were logged, under the thresholds then in force. It is
  not recomputed for the proposed threshold.
- Labels are taken as given. The id is not checked against a decision until the join, and a
  wrong label skews the scores.
