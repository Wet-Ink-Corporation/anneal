//! Integration tests for the `anneal` binary, compiled as one test binary (TST-02).

mod cli;
mod eval_routing;
mod hook_bootstrap;
mod hook_guard;
mod hooks_manifest;
mod ledger;
mod lint_skills;
mod lint_skills_inputs;
mod paths;
mod repo_gates;
#[cfg(test)]
mod support;
