"""Command-line entry point."""

import os

from app import audit, changed, config
from app.findings import collect
from app.tail import last

USAGE = """usage: app report <file>
       app tail <n> <file>
       app deleted-tests <diff-file>
       app delete <path>"""


def _read_lines(path: str) -> list[str]:
    with open(path, encoding="utf-8") as handle:
        return handle.read().splitlines()


def _report(path: str) -> int:
    settings = config.load(path)
    result = collect(_read_lines(path), marker=settings.marker)
    for finding in result.findings:
        print(finding)
    if result.omitted:
        print(f"... and {result.omitted} more")
    return 0


def _tail(count: str, path: str) -> int:
    if not count.isdigit():
        print(USAGE)
        return 2
    result = collect(_read_lines(path))
    for finding in last(result.findings, int(count)):
        print(finding)
    return 0


def _deleted_tests(path: str) -> int:
    for test in changed.deleted_tests(_read_lines(path)):
        print(test)
    return 0


def _delete(path: str) -> int:
    log = os.environ.get("APP_AUDIT_LOG", audit.DEFAULT_LOG)
    if not audit.record(log, "delete", path):
        print(f"refused: could not write the audit log {log}")
        return 1
    os.remove(path)
    print(f"deleted {path}")
    return 0


def main(argv: list[str]) -> int:
    match argv:
        case ["report", path]:
            return _report(path)
        case ["tail", count, path]:
            return _tail(count, path)
        case ["deleted-tests", path]:
            return _deleted_tests(path)
        case ["delete", path]:
            return _delete(path)
        case _:
            print(USAGE)
            return 2
