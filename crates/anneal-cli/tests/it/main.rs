//! Integration tests for the `anneal` binary, compiled as one test binary (TST-02).

// `ANNEAL_CLEF_URL`, which keeps these tests off the network, is honoured only with
// debug assertions (#28); a release-profile run would send fake tokens to Workers AI.
#[cfg(not(debug_assertions))]
compile_error!(
    "the integration tests need a debug build: ANNEAL_CLEF_URL is honoured only with debug_assertions"
);

mod brief;
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
mod hook_brief;
mod hook_completeness;
mod hook_fast_check;
mod hook_guard;
mod hook_guard_d1;
mod hook_handoff;
mod hook_injection_scan;
mod hook_merge_gate;
mod hook_prompt;
mod hook_standing_orders;
mod hook_status;
mod hooks_manifest;
mod ledger;
mod lint_plan;
mod lint_skills;
mod lint_skills_inputs;
mod lint_spec;
mod lint_terms;
mod migrate;
mod paths;
mod perf;
mod plan;
#[cfg(test)]
mod pr_repo;
mod preconditions;
mod release_check;
mod repo_gates;
mod repro;
mod rollout;
mod round;
mod ship;
mod statusline;
#[cfg(test)]
mod support;
mod task;
mod verdict;
// The fake `gh` is a shell script; `watch::tests` covers every exit code on every platform.
#[cfg(unix)]
mod watch;
