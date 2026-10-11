# Domain language

A grilling settles words as well as answers. When the human corrects a term, or two words turn
out to mean one thing, record it so specs, code and later sessions use the same word.

## Glossary entries

The glossary is `CONTEXT.md` at the repository root unless the project configures another path.
One entry per term:

```markdown
### Order
A customer's request to buy one or more items, from checkout until it ships or is cancelled.
_Avoid_: purchase, basket, cart order
```

- The heading is the term as the domain uses it, singular.
- The definition is one or two sentences in the human's words, with the boundary that tells the
  term from its neighbours.
- `_Avoid_:` lists the words that must not stand in for it, comma-separated. A scan of changed
  files reports each avoided term it finds, case-insensitively and as a whole word (#17), so
  list only words that are wrong in this domain.
- Add the entry when the human settles the term, not at the end of the grilling.

## Decisions worth an ADR

Most answers live in the design tree and the spec. Write an architecture decision record only
when a decision passes all three tests:

| Test | Question to ask |
|---|---|
| Hard to reverse | Would undoing it later cost a migration, a rewrite or a broken contract? |
| Surprising | Would a capable newcomer expect something else, and change it without the record? |
| A real trade-off | Were there at least two workable options, each giving something up? |

A decision that fails any test stays in the design tree. One that passes all three gets
`docs/adr/NNNN-<title-slug>.md`, numbered with the next free number, holding the context, the
options with what each gives up, the decision, and its consequences.
