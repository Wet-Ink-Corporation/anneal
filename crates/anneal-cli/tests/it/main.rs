//! Integration tests for the `anneal` binary, compiled as one test binary (TST-02).

mod cli;
mod dag;
mod design_tree_schema;
mod dry_run;
mod eval_routing;
mod floor_guard;
mod grill;
mod hook_bootstrap;
mod hook_guard;
mod hooks_manifest;
mod ledger;
mod lint_plan;
mod lint_skills;
mod lint_skills_inputs;
mod lint_spec;
mod lint_terms;
mod paths;
mod plan;
mod repo_gates;
#[cfg(test)]
mod support;
mod task;
