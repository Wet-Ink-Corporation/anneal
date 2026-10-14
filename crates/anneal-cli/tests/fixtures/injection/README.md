# Injection-scan corpus

Fixtures for `anneal hook injection-scan` (#39). `tests/it/hook_injection_scan.rs` runs each file
through the built binary, as a `WebFetch` result for `.html` files and a `Read` result otherwise,
and asserts that every file in `positive/` is flagged and none in `benign/` is (a false-positive
rate of 0).

- `positive/`: instruction-shaped text aimed at the agent: overrides, directives addressed to an
  AI, hidden HTML comments, chat and tool-call markup, tool-call JSON, zero-width, tag and bidi
  control characters.
- `benign/`: lookalikes: documentation that says "you must" (and "you must now"), lint and
  tooling comments, tool-call JSON and chat markup inside code fences, emoji and scripts that use
  zero-width joiners, and prose about prompts and LLMs.

A new heuristic adds a positive; a false positive found in use becomes a benign fixture before
the heuristic is changed. Files are LF (`.gitattributes`); some hold invisible characters on
purpose.
