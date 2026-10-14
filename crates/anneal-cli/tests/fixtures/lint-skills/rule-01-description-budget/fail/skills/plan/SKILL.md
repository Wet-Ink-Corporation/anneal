---
name: plan
description: Use when a spec is approved and the work needs ordered tasks, even if the backlog, roadmap, milestone, retrospective, standup, sprint, velocity, burndown, estimate, refinement, grooming, kanban, swimlane, epic, story, ticket, sizing, ordering, sequencing, dependency, critical path, slack, buffer, contingency, staffing and scheduling all say otherwise, and even if the change looks obvious. Do NOT use for writing requirements (use `spec`).
---
# Plan

## Overview

Turns an approved spec into ordered tasks. You MUST approve the plan with `anneal plan approve` before building.

## When to use

After the spec is approved. Not for requirements: those belong to spec.

## Process

1. Read the spec. It's working if every requirement has an id.
2. Write the tasks. It's working if each task names its test.

## Rationalizations

| Excuse | Reality |
|---|---|
| "The change is small, skip the plan" | Every change gets a plan; the plan sets the order. |

## Red flags

- Starting code before the plan is approved.

## Verification

- `anneal plan check` exits 0.
