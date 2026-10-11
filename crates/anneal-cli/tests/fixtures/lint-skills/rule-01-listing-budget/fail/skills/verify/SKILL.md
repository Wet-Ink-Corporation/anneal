---
name: verify
description: Use when verify000 verify001 verify002 verify003 verify004 verify005 verify006 verify007 verify008 verify009 verify010 verify011 verify012 verify013 verify014 verify015 verify016 verify017 verify018 verify019 verify020 verify0, even if it looks done. Do NOT use for other work (use `ship`).
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
