//! Integration tests for the `anneal` binary, compiled as one test binary (TST-02).

mod calibrate;
mod cli;
mod dag;
mod decide;
mod decide_http;
mod design_tree_schema;
mod dry_run;
mod eval_routing;
mod floor_guard;
mod grill;
mod hook_bootstrap;
mod hook_completeness;
mod hook_fast_check;
mod hook_guard;
mod hook_guard_d1;
mod hook_prompt;
mod hook_status;
mod hooks_manifest;
mod ledger;
mod lint_plan;
mod lint_skills;
mod lint_skills_inputs;
mod lint_spec;
mod lint_terms;
mod paths;
mod plan;
mod preconditions;
mod release_check;
mod repo_gates;
mod repro;
#[cfg(test)]
mod support;
mod task;
