"""Reads the files a unified diff (`git diff`) changes."""


def _path(header: str) -> str | None:
    """The path on a `--- a/<path>` or `+++ b/<path>` line, or None for /dev/null."""
    name = header[4:].rstrip("\n")
    if name == "/dev/null":
        return None
    return name[2:]


def deleted_tests(lines: list[str]) -> list[str]:
    """The test files (`tests/**/*.py`) the diff deletes."""
    deleted = []
    old = None
    for line in lines:
        if line.startswith("--- "):
            old = _path(line)
        elif line.startswith("+++ "):
            new = _path(line)
            if new is None and old and old.startswith("tests/") and old.endswith(".py"):
                deleted.append(old)
    return deleted
