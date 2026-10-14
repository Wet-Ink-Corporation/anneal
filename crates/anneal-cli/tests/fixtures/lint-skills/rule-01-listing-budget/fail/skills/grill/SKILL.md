---
name: grill
description: Use when grill000 grill001 grill002 grill003 grill004 grill005 grill006 grill007 grill008 grill009 grill010 grill011 grill012 grill013 grill014 grill015 grill016 grill017 grill018 grill019 grill020 grill021 grill022 grill023 g, even if it looks done. Do NOT use for other work (use `spec`).
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
