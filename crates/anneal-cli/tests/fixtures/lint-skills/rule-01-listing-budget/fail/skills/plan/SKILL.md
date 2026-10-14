---
name: plan
description: Use when plan000 plan001 plan002 plan003 plan004 plan005 plan006 plan007 plan008 plan009 plan010 plan011 plan012 plan013 plan014 plan015 plan016 plan017 plan018 plan019 plan020 plan021 plan022 plan023 plan024 plan025 plan026, even if it looks done. Do NOT use for other work (use `build`).
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
