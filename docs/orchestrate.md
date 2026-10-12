# Orchestration: fan-out, Clef D2 and the weigh-in bridge

`/anneal:orchestrate` (`plugin/skills/orchestrate/SKILL.md`, user-invoked because fanning out
costs about 15 times the tokens of one session) runs an approved plan's ready tickets across
subagents. Its deterministic steps are `anneal orchestrate`, `anneal decide --point D2`, and the
brief and round commands of [brief.md](brief.md).

Rulings, not stalls: a reversible call is ruled on and recorded; a one-way call is parked in
weigh-in with a default, and the run moves on. anneal never stores a decision itself: weigh-in
owns parked items and their rulings, and anneal records only that it parked, that the bridge
was unavailable, and the orchestrator's own rulings.

```text
anneal orchestrate fan-out [--tracker markdown|github] [--dir PATH] [--repo OWNER/NAME] [--issue N]
anneal orchestrate park --ticket <id> --q <question> --default <option> --flip <fact>
                        --at <path:line> --resolve-by <YYYY-MM-DD> [--did <text>]
                        [--decision <D2 id>] [--session <id>]
anneal orchestrate rule --ticket <id> --what <text> --why <text> --cost <text>
                        [--decision <D2 id>] [--session <id>]
```

Exit codes: 0 fan out (park: parked; rule: recorded), 3 `fan-out`: fewer than 3 independent
tickets, 4 `park`: weigh-in unavailable, rule instead; 2 for refused arguments, 70 when the
ledger or the tickets cannot be read ([exit-codes.md](exit-codes.md)).

## `fan-out`

It reads the tickets as `anneal dag frontier` does (same tracker flags, [dag.md](dag.md)) and
exits 0 only when the frontier holds 3 or more tickets. Frontier tickets are independent: each
has every blocker done, so none waits on another. Otherwise it exits 3 with an
`orchestrate/no-fan-out` finding, and the skill builds the tickets one at a time. A graph that
fails its check (a cycle, a dangling reference) lists no tickets, so it is refused too, with
the check's findings. `data` is `{tracker, fan_out, independent, required, tickets}`. It writes
nothing.

## Clef D2 and the call

For each judgment call the skill asks `anneal decide --point D2 --question noul --state <call>`
([decide.md](decide.md)): `one_way_door` at 0.50 or above is `park`, below it `rule`. D2 starts
in shadow, where it logs only: the skill parks only on `park` with `data.enforce: true`, and
rules otherwise (shadow, `rule`, Clef off with exit 3, no answer with exit 5, or `park` exit 4).

## `park`: the weigh-in bridge

**Finding `weighin`.** weigh-in ships its CLI inside its own plugin
(`${CLAUDE_PLUGIN_ROOT}/libexec/weighin` there), which anneal cannot see. anneal uses
`$WEIGHIN_BIN` when it is set and not empty (it must name a file), else the first `weighin` on
`PATH` (`weighin.exe`, `weighin.cmd` or `weighin.bat` on Windows).

**Parking.** Every argument is checked first (the ticket as `anneal round` takes it; each text
one non-blank line of at most 1,024 characters for `--q` and 512 for the rest; `--resolve-by` a
`YYYY-MM-DD` date; `--decision` a decision id; `--session` a ledger tag), then the ledger is
opened, then `weighin batch` runs in the working directory with one JSON array on stdin:

```json
[
  {"op": "raise", "q": "<question>", "door": "one-way", "authority": "authority",
   "did": "<--did, default blocked>", "flip": "<fact>", "at": "<path:line>", "blocks": "<ticket>"},
  {"op": "defer", "id": "@1", "flip": "<fact>", "trigger": "next sitting",
   "resolve_by": "<date>", "default": "<option>"}
]
```

It gets 10 s, and each of its output streams is capped at 4 MiB (a longer one is a failure,
not read in part). The receipt is read line by line by its first words: `raised wi-…` or
`wi-… already raised`, and `deferred wi-…` for the same item. Raising is idempotent in weigh-in,
so a park run again (a retried hook, a resumed session) names the same item; the ledger gets
one `weighin_parked {item_id, ticket, decision?}` per item and ticket, and a repeat reports
`data.recorded: false`.

**Failing open.** With no `weighin`, or when it cannot start, times out, exits non-zero, writes
too much, or prints a receipt that does not show the item raised and deferred, the park records
`bridge.unavailable {ticket, reason}` and exits 4 with an `orchestrate/bridge-unavailable`
finding: the orchestrator rules on the call instead. A partial batch (raised, default refused)
is such a failure; running the park again finishes it.

anneal never writes `weigh-in` marker blocks for a parked call: weigh-in's Stop hook would
capture them too and raise the call twice.

## `rule`

Records `orchestrate.ruled {ticket, what, why, cost_if_wrong, decision?}`: what was ruled, why,
and what undoing it costs if the ruling is wrong. Nothing is asked.

## Reading rulings back (D6)

The `ruled-before` hook offers Clef D6 the `.out-of-scope/` rulings, weigh-in's accepted
decision atoms (`.kb/_intake/decisions/*.md`), and the items `weighin open --all --json` lists
as decided whose atoms are not written yet ([hooks.md](hooks.md)). A ruling recorded with
`weighin decide` on a parked item is therefore surfaced on a later prompt.

Labelling the D2 decision from that ruling (the default held, or the human chose otherwise) is
not automatic yet: `weighin_parked` keeps the D2 decision id so it can be, and until then it is
labelled by hand with `anneal decide label <id> <outcome>`.

## Tests against weigh-in

Unit tests drive the bridge through its `Runner` port with a fake on every platform; the
integration tests run the binary with weigh-in absent (every platform) and with a fake
`weighin` shell script (unix). `orchestrate::the_pinned_weigh_in_parks_once_and_its_ruling_reaches_d6`
runs the release exit criterion against a real weigh-in when `WEIGHIN_BIN` names one (0.6.0 or
later; the CI step that downloads a pinned release and sets it is still to be added); without
it the test prints that it skipped.
