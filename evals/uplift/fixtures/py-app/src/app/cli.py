"""Command-line entry point."""

from app.findings import collect


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[0] != "report":
        print("usage: app report <file>")
        return 2
    with open(argv[1], encoding="utf-8") as handle:
        result = collect(handle.read().splitlines())
    for finding in result.findings:
        print(finding)
    if result.omitted:
        print(f"... and {result.omitted} more")
    return 0
