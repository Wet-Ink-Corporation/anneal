# Agent charter

All Rust here follows the standard. You must run the tests before you commit, and you must not
weaken a lint to get green.

## Claude Code specifics

- The plugin enforces the standard. You must load the skill before you edit Rust.
- Every change reaches main through a pull request with green CI.
- Agents must record decisions in the decisions log.
