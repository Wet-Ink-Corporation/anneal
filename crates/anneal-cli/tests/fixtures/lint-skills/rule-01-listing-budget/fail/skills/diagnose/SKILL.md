---
name: diagnose
description: Use when diagnose000 diagnose001 diagnose002 diagnose003 diagnose004 diagnose005 diagnose006 diagnose007 diagnose008 diagnose009 diagnose010 diagnose011 diagnose012 diagnose013 diagnose014 diagnose015 diagnose016 diagnose017, even if it looks done. Do NOT use for other work (use `verify`).
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
