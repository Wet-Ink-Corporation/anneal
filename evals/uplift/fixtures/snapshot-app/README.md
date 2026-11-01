# snapshot-app

Reads report snapshots from disk. `snapshot.read_snapshot(path)` returns a snapshot file's bytes, and refuses a file over the cap, `MAX_BYTES` (1 MiB), by raising `SnapshotTooLarge`, so a runaway snapshot cannot fill memory.

The tests use the standard library's `unittest` only: run them with `python3 -m unittest`. `pytest` is not installed.
