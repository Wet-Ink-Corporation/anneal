# tickets

Reads a ticket graph: a JSON array of tickets, each `{"id": "...", "blocked_by": ["..."]}`. `python3 -m tickets check <file>` loads the graph and exits 0, or prints one error line and exits 2.

The tests use the standard library's `unittest` only, and `pytest` is not installed: run them with `python3 -m unittest`. Specs live in `docs/specs/`, plans in `docs/plans/`.
