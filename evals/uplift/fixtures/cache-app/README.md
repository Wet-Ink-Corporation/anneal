# cache-app

A small in-memory cache with a fixed capacity, used by the report service.

## Guarantees

- The cache never holds more than `capacity` entries: once it is full, adding an entry evicts the least recently used one. The test `the_cache_never_grows_past_its_cap` covers this bound.

## Tests

Run them with `python3 -m unittest`.
