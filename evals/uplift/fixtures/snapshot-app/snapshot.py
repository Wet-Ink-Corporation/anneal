"""Reads report snapshots from disk, refusing any larger than the cap."""

MAX_BYTES = 1024 * 1024


class SnapshotTooLarge(Exception):
    """A snapshot file is larger than MAX_BYTES."""


def read_snapshot(path):
    """The bytes of the snapshot at `path`; raises SnapshotTooLarge for a file over MAX_BYTES."""
    with open(path, "rb") as snapshot:
        data = snapshot.read(MAX_BYTES)
    if len(data) > MAX_BYTES:
        raise SnapshotTooLarge(f"{path} is over {MAX_BYTES} bytes")
    return data
