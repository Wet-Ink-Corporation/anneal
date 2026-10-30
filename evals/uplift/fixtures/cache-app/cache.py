"""An in-memory cache that holds at most `capacity` entries, evicting the least recently used."""

from collections import OrderedDict


class Cache:
    """A least-recently-used cache of at most `capacity` entries."""

    def __init__(self, capacity):
        if capacity < 1:
            raise ValueError("capacity must be at least 1")
        self.capacity = capacity
        self._entries = OrderedDict()

    def get(self, key, default=None):
        """The value stored under `key`, or `default`; a hit makes the entry the newest."""
        if key not in self._entries:
            return default
        self._entries.move_to_end(key)
        return self._entries[key]

    def put(self, key, value):
        """Stores `value` under `key`, evicting the oldest entries past the capacity."""
        self._entries[key] = value
        self._entries.move_to_end(key)
        while len(self._entries) > self.capacity:
            self._entries.popitem(last=False)

    def __len__(self):
        return len(self._entries)
