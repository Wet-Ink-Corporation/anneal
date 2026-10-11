# `anneal decide`

`anneal decide --point <D1..D7> --question <noul|choice|score> --state <json> [--option <o>]...
[--thresholds <file>]` asks Clef one question for a decision point, applies the point's
thresholds to the probability Clef returns, and appends one row to the decision log.
`anneal decide label <id> <outcome>` attaches an outcome label to a logged decision.

The hooks that call it (D1 in the guard, D3 and D6 on prompts, D7 in `grill`) land with #30,
#31 and #32; this command is the one choke point they share.

## Transport

Clef runs on Workers AI and is called directly:
`POST https://api.cloudflare.com/client/v4/accounts/{account}/ai/run/{model}` with
`Authorization: Bearer <token>`, a 2 s timeout for the whole call. The request is Workers AI's
chat shape, stable part first, so a prefix cache can reuse it:

```json
{"messages": [
  {"role": "system", "content": "{\"point\":\"D1\",\"question\":\"noul\",\"subject\":\"destructive_or_irreversible\",\"options\":[\"yes\",\"no\"]}"},
  {"role": "user", "content": "<the compact state>"}
]}
```

The response is read tolerantly: Workers AI's `{"result": {"response": ...}}` with the model's
JSON as text or as an object, holding `{"probabilities": {<option>: p}}` (noul: `yes` and `no`;
choice: each `--option`) or `{"score": v}`. Unknown fields, Clef's `confidence` among them, are
ignored. A missing probability, a number outside 0..=1, `success: false` or a body that is not
JSON is a bad response, and so are noul or choice probabilities that do not sum to 1 within
0.05 (noul's `no` may be left out, as 1 - `yes`; when given, it counts).

**No HTTP client yet.** Adding one is a pending dependency decision (H-16; ureq 3 with rustls is
the recommended option), so the binary is built with a transport that reports every call as
unreachable ("no HTTP client configured (pending dependency decision)"). Until it lands,
`decide` with an account always exits 5 and every caller takes its fallback. The network sits
behind the `Transport` port (`decide::transport`), so the adapter is one new module wired in
`cli`.

## Configuration

The plugin's `userConfig` reaches hooks as environment variables. Claude Code (checked in
2.1.296) names them `CLAUDE_PLUGIN_OPTION_<KEY>` with the key upper-cased, and passes the
sensitive token too:

| Variable | `userConfig` key | Meaning |
|---|---|---|
| `CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID` | `cf_account_id` | Cloudflare account (1 to 64 letters and digits). Empty or unset: Clef is off. |
| `CLAUDE_PLUGIN_OPTION_CF_API_TOKEN` | `cf_api_token` | Workers AI token. Unset with an account set: every call is unreachable. |
| `CLAUDE_PLUGIN_OPTION_CLEF_MODE` | `clef_mode` | `off`, `shadow` or `enforce`, then optional per-point overrides: `shadow,D1=off,D3=enforce`. Empty or unset: `shadow`. |

The token is never written anywhere: it has no `Display`, its `Debug` is `<redacted>`, and no
error, envelope or row carries it (tested).

A malformed value (an account that is not letters and digits, a token that is not printable
ASCII without spaces, a mode that does not parse, or a variable that is not UTF-8) does not
stop the hook with an internal error: the point takes its fallback (exit 5) with
`data.error: "misconfigured"` and `data.reason` naming the variable, never its token value.
Nothing is asked and nothing is logged.

## Thresholds

`plugin/playbooks/clef-thresholds.json` holds each point's model alias, its fallback and, per
question type, a subject and bands. The file is found from `--thresholds`, else
`$CLAUDE_PLUGIN_ROOT/playbooks/clef-thresholds.json`, else the copy built into the binary; a
file named either way must exist and be a regular file. It is read strictly (version 1, every
point present, unknown fields refused); a model id is 1 to 128 of `A-Z a-z 0-9 @ / . _ : -`
with no empty, `.` or `..` path segment, so it cannot change the URL path it is joined to.

A rule applies to a probability, never to `confidence`: noul uses the probability of `yes`,
choice the top option's (the first given wins a tie), score the value. The first band whose
`at_least` the probability reaches gives the action; otherwise `otherwise` does. Defaults, from
the blueprint:

| Point | Model | Question | Bands | Otherwise | Fallback |
|---|---|---|---|---|---|
| D1 guard | clef-flash | noul `destructive_or_irreversible` | 0.90 deny, 0.40 ask | defer | **ask** |
| D2 orchestrate | clef-flash | noul `one_way_door` | 0.50 park | rule | fail_open |
| D3 route | clef-flash | choice `phase`; score `size` | 0.60 hint; none | none; record | fail_open |
| D4 completeness | **clef** | noul `present_and_grounded` | 0.50 pass | block | fail_open |
| D5 verify | clef-flash | score `defect_risk` | 0.6666 doubt_review | skip | fail_open |
| D6 ruled-before | clef-flash | choice `ruled_before` | 0.70 surface | none | fail_open |
| D7 grill | clef-flash | noul `is_decision` | 0.50 ask_human | find_fact | fail_open |

Model aliases map to Workers AI ids in the file's `models` table (`clef-flash`, `clef`), so a
model can change without a code change. The ids shipped are placeholders until the deployed
models are named. D5's blueprint rule, an expected level of 2.0 or more on low/med/high/critical
(0 to 3), is the normalised score 2/3, written 0.6666.

## Modes

The mode of a point is its override, else the global mode. `off` (or no account) asks nothing
and logs nothing: exit 3, and the caller uses its deterministic layer; the envelope gives
`data.reason` (`mode_off` or `no_account`) and the point's `data.configured_mode`. `shadow` asks, logs and
reports the action with `data.enforce: false`; callers do not act on it. `enforce` reports
`data.enforce: true`, except that a point stays in shadow (`data.mode: "shadow"`,
`data.configured_mode: "enforce"`) until it has 50 answered decisions. The count is kept in
`decisions/counts.json`, so rotation does not reset it; two decisions at once can lose an
increment, which only lengthens the shadow period. The file is not protected: editing it by
hand to raise a point's count skips (part of) the shadow period, so `enforce` takes effect
early; deleting it only restarts the count, which lengthens the shadow period.

## State

`--state` is a JSON object of at most 64 KiB. It is compacted before it is sent and hashed:
`tool`, `phase`, `input` first, then the other fields by name, then the volatile ones (`sha`,
`commit`, `head`, `timestamp`, `ts`, `time`, `date`, `counter`, `count`, `turn`, `seq`,
`session_id`, `nonce`, or a name ending in `_sha`, `_at`, `_ts`, `_time`, `_count`, `_counter`)
by name. Every string is trimmed and cut to 1,024 characters (marked `…`); the result must be
16 KiB or less. Anything else is a usage error.

## Exit codes

| Exit | Meaning | Log |
|---|---|---|
| 0 | Clef answered; `data.action` is the thresholds' action (`decide label`: the label was appended) | one decision row (label row) |
| 3 | Clef is off for the point (no account, or mode `off`) | nothing |
| 5 | no answer: unreachable, timed out (2 s), or a bad response; `data.fallback` is the point's fallback (`ask` for D1, `fail_open` for the others) | one row with `error` and `unreachable` |
| 5 | the account, token or mode is malformed: `data.error` is `misconfigured`, `data.reason` names the variable, `data.fallback` as above | nothing |
| 2 | usage error: bad arguments, a state that is not a JSON object or too large, a question the point does not ask, options that do not fit, a malformed label | nothing |
| 70 | the thresholds file, log or clock could not be read or written | nothing |

## The decision log

`~/.anneal/decisions/decisions.jsonl` (`$ANNEAL_HOME/decisions/` when set). It is outside the
plugin's directories, so it survives a reinstall; `CLAUDE_PLUGIN_DATA` is deleted on uninstall.
Rows are appended, never rewritten. A decision row:

```json
{"kind": "decision", "id": "d1760000000000-4242-0", "ts_ms": 1760000000000,
 "point": "D1", "question": "noul", "subject": "destructive_or_irreversible",
 "model": "@cf/wet-ink/clef-flash", "state_sha256": "sha256:…",
 "probabilities": {"yes": 0.95, "no": 0.05}, "top": null, "probability": 0.95,
 "thresholds": {"subject": "…", "bands": [{"at_least": 0.9, "action": "deny"}, …], "otherwise": "defer"},
 "threshold": 0.9, "action": "deny", "mode": "shadow", "configured_mode": "shadow",
 "latency_ms": 39, "outcome": null, "unreachable": false, "error": null, "error_reason": null}
```

Without an answer, `probabilities`, `probability`, `threshold` and `action` are null, `error` is
`unreachable`, `timed_out` or `bad_response`, `unreachable` is true for the first two, and
`error_reason` says why (at most 256 characters). A label row is
`{"kind": "label", "id", "outcome", "ts_ms"}`; the decision's `outcome: null` slot is filled by
joining on `id`, never by rewriting it. A label's id must have the shape `decide` writes
(`d<millis>-<pid>-<n>`) but is not looked up; an outcome is 1 to 64 of `A-Z a-z 0-9 _ . : -`.

Bounds: a row is at most 8 KiB: a choice has 2 to 32 distinct options, each 1 to 64 of
`A-Z a-z 0-9 _ . : -` (the outcome-label alphabet, so any option can be a label), none of which
JSON escapes, and a test keeps the largest such row under the cap. When the live file would
pass 16 MiB it becomes `decisions.1.jsonl`, the older files move up to `decisions.4.jsonl` and
the fifth is dropped, so the log holds at most 80 MiB; when two calls rotate at once, the one
that finds the live file already gone carries on and appends to a new one. `anneal calibrate`
(#29) has to read the rotated files as well as the live one.
