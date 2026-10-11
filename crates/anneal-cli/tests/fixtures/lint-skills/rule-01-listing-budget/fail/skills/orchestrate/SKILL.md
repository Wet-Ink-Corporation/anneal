---
name: orchestrate
description: Use when orchestrate000 orchestrate001 orchestrate002 orchestrate003 orchestrate004 orchestrate005 orchestrate006 orchestrate007 orchestrate008 orchestrate009 orchestrate010 orchestrate011 orchestrate012 orchestrate013 orches, even if it looks done. Do NOT use for other work (use `grill`).
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
