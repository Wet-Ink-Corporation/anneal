"""The last entries of a list."""


def last(items: list[str], count: int) -> list[str]:
    """The last `count` entries of `items`, oldest first."""
    return items[-count:]
