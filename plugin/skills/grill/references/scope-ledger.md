# Scope ledger

When the human rules that something is out of scope, write the ruling down so the next session
does not ask again, and so later checks can cite it instead of reopening it.

## One file per ruling

Rulings live in `.out-of-scope/` at the repository root, one Markdown file per ruling, named by
a short slug: `.out-of-scope/<slug>.md`.

```markdown
---
id: csv-export-excel-formulas
ruled_at: 2026-10-11
summary: CSV export writes plain values; Excel formulas are out of scope.
weigh_in_ref: wi-2415fc
---

Asked during the grilling of the order export: should totals export as formulas? The human
ruled no: plain values only, because the files feed an import job, not people.
```

- `id` matches the file name without `.md` and never changes once written.
- `ruled_at` is the date the human ruled.
- `summary` is one line: what is out of scope.
- `weigh_in_ref` is optional: the weigh-in decision the ruling came from, when there is one.
- The body says what was asked and why the human ruled as they did.

A ruling is one decision. If the human later brings the thing back into scope, write a new
ruling that names the old id rather than editing the old file.

## Who owns decisions

weigh-in owns decisions: when a weigh-in decision exists for the question, cite it in
`weigh_in_ref` and do not re-decide it here. The ruling store records what was ruled out of
scope for this work; it does not make or override decisions. In the design tree, mark the node
`deferred` with the ruling's `id` in its `answer`.
