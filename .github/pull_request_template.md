## What changed

<!-- Before / After, in plain words. Link the story it closes (`Closes #N`). -->

## Acceptance criteria

<!-- Copy the story's checklist; tick each one with its evidence (a test name, a CI step, command output). -->

## Gate

- [ ] `cargo fmt --check`, `node .claude/temper/temper.mjs baseline --check`, `cargo test --workspace`
- [ ] `claude plugin validate --strict .` and `scripts/check-versions.sh`
- [ ] `temper:rust-reviewer` verdict `approved` on the head tree

## Decisions

<!-- Weigh-In items (wi-…) taken or touched, and where they are recorded. "None" is fine. -->
