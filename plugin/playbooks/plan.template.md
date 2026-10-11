# Plan: <feature>

<!--
Copy this file beside the spec's plans (for example `docs/plans/<feature>.md`), replace every
<placeholder>, and run `anneal lint plan <file>` until it exits 0. The lint checks:
- a `Spec:` line before the first `##` section names the spec, and the file exists (read
  relative to this plan's directory, as a markdown link is);
- the `##` sections Goal, Tasks and Review focus are present once each, Goal and Review focus
  not empty (Architecture is optional);
- Tasks holds at least one `### <id>: <title>` task (or `### Task <id>: <title>`), ids unique;
- every task has one `Files:` line with each file in backticks, one `Test:` line with the
  command in backticks, and one `Blocked by:` line: `none`, or ids of other tasks;
- no task is blocked by itself, by an id that is not a task, or through a cycle.
Then the human approves it: `anneal plan approve <file>` refuses while the spec has any
[NEEDS CLARIFICATION] marker, and `anneal plan check <file>` exits 4 once the plan changes.
-->

- Spec: [<feature>](../specs/feature.md)

## Goal

<One sentence: what the user can do when every task is done.>

## Architecture

<Two or three sentences: the approach, where the code goes, what it reuses.>

## Tasks

<!--
One thin end-to-end slice per task, in build order. Steps under a task are test-first:
write the failing test, run the test command and see it fail, write the code, run it green.
-->

### T1: <first tracer-bullet slice>

- Files: `<path/to/code>`, `<path/to/test>`
- Test: `<the command that runs this task's tests>`
- Blocked by: none

1. Write the failing test for <behaviour>.
2. Run the test command; it fails because <reason>.
3. Write the code; run the test command; it passes.

### T2: <next slice>

- Files: `<path/to/code>`
- Test: `<the command that runs this task's tests>`
- Blocked by: T1

1. <Step.>

## Review focus

- <Where a reviewer should look hardest, and why: the risky change, the subtle invariant.>
