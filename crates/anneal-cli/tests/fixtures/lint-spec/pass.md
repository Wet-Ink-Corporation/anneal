# Spec: offline export

## Problem

Analysts lose work when the connection drops during an export.

## Users

Analysts who export reports from the field.

## Requirements

- Exports survive a dropped connection.
- Exports can be resumed.

## Acceptance criteria

- THE exporter SHALL write UTF-8.
- WHEN the user asks for an export, THE exporter SHALL write a CSV file.
- WHILE offline, THE exporter SHALL queue each export.
- IF the disk is full, THEN THE exporter SHALL keep the partial file and say so.
- WHERE resumable exports are enabled, THE exporter SHALL resume from the last row.
- WHERE resumable exports are enabled, WHILE offline, WHEN the link returns,
  THE exporter SHALL resume every queued export.
- **AC-7**: WHILE uploading IF the link drops THEN THE exporter SHALL retry once.
- [x] AC-8: WHEN the user picks the folder where the file goes, THE exporter SHALL then save it.

```text
- not a criterion: code blocks are skipped
```

## Out of Scope

- PDF export, because no user asked for it.

## Open questions

- Which delimiter? [NEEDS CLARIFICATION: comma or semicolon]
- `[NEEDS CLARIFICATION]` in code does not count.

## Constitution check

Constitution version: 3

- [G1] pass
- [G2] violated: the queue is kept on disk beside the ledger.
- [G3] n/a

### Complexity

| Gate | Violation | Justification |
|---|---|---|
| G2 | a disk queue beside the ledger | the ledger is unreachable while offline |
