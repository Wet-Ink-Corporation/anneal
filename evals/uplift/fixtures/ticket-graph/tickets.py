"""Loads a ticket graph from a JSON file."""

import json
import sys
from dataclasses import dataclass, field


@dataclass
class Ticket:
    id: str
    blocked_by: list[str] = field(default_factory=list)


class GraphError(Exception):
    """The file is not a valid ticket graph."""


def load(path: str) -> list[Ticket]:
    """The tickets in the file at `path`, in file order."""
    try:
        with open(path, encoding="utf-8") as handle:
            raw = json.load(handle)
    except (OSError, ValueError) as error:
        raise GraphError(f"cannot read {path}: {error}") from error
    if not isinstance(raw, list):
        raise GraphError(f"{path} is not a JSON array")
    return [Ticket(id=str(item["id"]), blocked_by=list(item.get("blocked_by", []))) for item in raw]


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[0] != "check":
        print("usage: tickets check <file>")
        return 2
    try:
        load(argv[1])
    except GraphError as error:
        print(error)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
