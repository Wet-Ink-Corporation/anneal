# Spec: <feature>

<!--
Copy this file to the spec's place (for example `docs/specs/<feature>.md`), replace every
<placeholder>, and run `anneal lint spec <file>` until it exits 0. The lint checks:
- every `##` section below is present (any letter case);
- every list item under Acceptance criteria is EARS (grammar in docs/spec.md);
- Out of Scope is not empty;
- at most 3 [NEEDS CLARIFICATION] markers; plan approval needs none;
- the Constitution check names constitution.md's version, lists every gate, and justifies
  each violated gate in the Complexity table;
- no term the glossary (CONTEXT.md) lists under `_Avoid_:`.
Write the spec only from requirements the human confirmed; mark what is still open with
`[NEEDS CLARIFICATION: <question>]` instead of guessing.
-->

## Problem

<Who has what problem today, and what it costs them. No solution here.>

## Users

<Who uses this, and what they are trying to get done.>

## Requirements

- <What the system must do, one item each, in the users' words.>

## Acceptance criteria

<!--
One list item per criterion. Keywords are in capitals; the clauses come in this order:
  THE <system> SHALL <response>.                                   (ubiquitous)
  WHEN <trigger>, THE <system> SHALL <response>.                   (event-driven)
  WHILE <state>, THE <system> SHALL <response>.                    (state-driven)
  IF <condition>, THEN THE <system> SHALL <response>.              (unwanted behaviour)
  WHERE <feature>, THE <system> SHALL <response>.                  (optional feature)
  WHERE <feature>, WHILE <state>, WHEN <trigger>, THE <system> SHALL <response>.  (complex)
A label (`**AC-1**:` or `AC-1:`) or a checkbox may come first.
-->

- WHEN <trigger>, THE <system> SHALL <response>.
- IF <unwanted condition>, THEN THE <system> SHALL <response>.

## Out of Scope

- <What this spec will not do, and why.>

## Open questions

- <A question the human still has to answer, or "None.">

## Constitution check

<!--
Read constitution.md. Name its version, then list every gate with `pass`, `violated: <why>`
or `n/a`. Each violated gate needs a row in the Complexity table saying what breaks the gate
and why the simpler option is not enough. Without a constitution.md, keep the section and
write "No constitution." in it.
-->

Constitution version: <N>

- [<gate id>] pass

### Complexity

| Gate | Violation | Justification |
|---|---|---|
