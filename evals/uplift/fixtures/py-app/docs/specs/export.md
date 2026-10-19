# Spec: export

Status: finished (requirements agreed 2026-10-10)

## Acceptance criteria

1. WHEN the user runs `app export <file> --format csv`, the tool SHALL write one CSV row per finding with the columns `line` and `text`.
2. WHEN `--format json` is given, the tool SHALL write a JSON array of `{"line", "text"}` objects.
3. IF the output file already exists, THEN the tool SHALL refuse with exit code 2 unless `--force` is given.

## Out of Scope

- Streaming output for files larger than memory.
