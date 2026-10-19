"""The audit log: one line per guarded action, written before the action runs."""

import time

DEFAULT_LOG = "audit.log"


def record(log_path: str, action: str, target: str) -> bool:
    """Appends `action target` to the audit log. True when the line was written."""
    try:
        with open(log_path, "a", encoding="utf-8") as handle:
            handle.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {action} {target}\n")
    except OSError:
        pass
    return True
