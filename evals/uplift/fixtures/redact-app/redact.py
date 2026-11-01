"""Hides secrets in command lines before they are logged."""

import re
import shlex

MASK = "***"

# The options whose value is a secret, per command: short options without their dash.
SECRET_OPTIONS = {
    "curl": {"--user", "--oauth2-bearer"},
    "mysql": {"p", "--password"},
}

_ASSIGNMENT = re.compile(r"\b([A-Z][A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|KEY))=\S+")


def _secret_option(command: str, word: str) -> bool:
    """Whether `word` is an option of `command` whose next word is a secret."""
    options = SECRET_OPTIONS.get(command, set())
    if word.startswith("--"):
        return word in options
    return word.startswith("-") and word[1:] in options


def redact(line: str) -> str:
    """`line` with every secret it holds replaced by MASK."""
    line = _ASSIGNMENT.sub(lambda match: f"{match.group(1)}={MASK}", line)
    words = shlex.split(line)
    if not words:
        return line
    out = [words[0]]
    hide_next = False
    for word in words[1:]:
        if hide_next:
            out.append(MASK)
            hide_next = False
        else:
            out.append(word)
            hide_next = _secret_option(words[0], word)
    return shlex.join(out)
