# `eval uplift` test fixtures

The plugin is `../eval-routing/` (its root holds `.claude-plugin/plugin.json`, named `fixture`,
and `skills/`). `suites/` is a valid Tier 3 suite set: `plan` has a regression and a
capability case, `spec` a guard case. Every case has the graders `outcome` and `always`, so
the fake `claude` in `tests/it/eval_uplift.rs` can answer by name. `invalid/` has one case
whose `llm` grader comes first.
