# Spec: notes list --json

## Problem

People who script their notes have to parse the human table that `notes list` prints. The
table's column widths change with the titles, so their scripts break whenever a long title
appears.

## Users

People scripting their notes in shell pipelines and CI jobs, who want to pipe the list of
notes into `jq` or a script.

## Requirements

- `notes list --json` prints the notes as JSON instead of the table.
- The JSON is one array, one object per note, with its id, title and created time.
- With no notes, it prints an empty array.
- Errors stay as they are: one line on stderr, nothing on stdout.

## Acceptance criteria

- **AC-1**: WHEN the user runs `notes list --json`, THE notes CLI SHALL print one JSON array on stdout with one object per note holding `id`, `title` and `created`.
- **AC-2**: WHEN the user runs `notes list --json`, THE notes CLI SHALL sort the array by `id` and write `created` in RFC 3339.
- **AC-3**: WHEN the user runs `notes list --json` with no notes, THE notes CLI SHALL print `[]` and exit 0.
- **AC-4**: IF the notes file cannot be read while `--json` is set, THEN THE notes CLI SHALL print one line on stderr, nothing on stdout, and exit 1.
- **AC-5**: THE notes CLI SHALL print the existing table when `--json` is not set.

## Out of Scope

- `--json` on `notes show` and the other commands: only `notes list` was asked for.
- A `--pretty` option for indented JSON: deferred; `jq .` indents it.
- A JSON schema file for the output: no consumer needs one yet.

## Open questions

- None.

## Constitution check

Constitution version: 1

- [G1] pass
- [G2] pass
- [G3] pass

### Complexity

| Gate | Violation | Justification |
|---|---|---|
