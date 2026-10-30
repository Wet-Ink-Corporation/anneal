"""Collects findings from log lines, keeping at most MAX_FINDINGS."""

from dataclasses import dataclass, field

MAX_FINDINGS = 1000


@dataclass
class Findings:
    findings: list[str] = field(default_factory=list)
    omitted: int = 0


def collect(lines: list[str], marker: str = "ERROR") -> Findings:
    result = Findings()
    for line in lines:
        if marker not in line:
            continue
        if len(result.findings) < MAX_FINDINGS:
            result.findings.append(line)
        else:
            result.omitted += 1
    return result
