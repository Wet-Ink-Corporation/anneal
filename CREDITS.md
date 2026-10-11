# Credits

anneal re-authors techniques from other projects rather than vendoring them. Each borrowed technique is listed here.

| Technique | Source repository | Licence |
| --- | --- | --- |
| Grilling: walk a plan's design tree one branch at a time until shared understanding, looking up in the codebase what it can answer (the `grill` skill's frontier rounds and design tree) | [mattpocock/skills](https://github.com/mattpocock/skills) (`grill-me`) | MIT |
| Interviewing the human with a recommended answer for each question before building (the `grill` skill's recommended answers and builder check) | [addyosmani/agent-skills](https://github.com/addyosmani/agent-skills) (`interview-me`) | MIT |
| Brainstorming: one question at a time, then restating the design for the human to validate before work starts (the `grill` skill's restate gate) | [obra/superpowers](https://github.com/obra/superpowers) (`brainstorming`) | MIT |
| Writing plans: a header with the goal and architecture, bite-sized tasks each naming its exact files and the command that runs its test, test-first steps, and a review focus for the reviewer (the `plan` skill and `playbooks/plan.template.md`) | [obra/superpowers](https://github.com/obra/superpowers) (`writing-plans`) | MIT |
| Checking a plan's structure with a script before it is used: required sections, per-task fields and blocking edges, each failure with a fix (`anneal lint plan`) | pstack (`check-plan.mjs`); repository link to be added | not yet confirmed |
| Diagnosing bugs: build a loop that shows the failure before anything else, rank falsifiable hypotheses, and tag temporary instrumentation so one search removes it (the `diagnose` skill, `anneal repro record` and `anneal repro leftovers`) | [mattpocock/skills](https://github.com/mattpocock/skills) (`diagnosing-bugs`) | MIT |
| Systematic debugging: no fix before the root cause, read the evidence and compare with a working case, test one hypothesis at a time, and question the design after repeated failed fixes (the `diagnose` skill's process, rationalizations and red flags) | [obra/superpowers](https://github.com/obra/superpowers) (`systematic-debugging`) | MIT |
