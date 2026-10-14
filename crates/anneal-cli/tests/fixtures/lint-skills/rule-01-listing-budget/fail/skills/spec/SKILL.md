---
name: spec
description: Use when spec000 spec001 spec002 spec003 spec004 spec005 spec006 spec007 spec008 spec009 spec010 spec011 spec012 spec013 spec014 spec015 spec016 spec017 spec018 spec019 spec020 spec021 spec022 spec023 spec024 spec025 spec026 s, even if it looks done. Do NOT use for other work (use `plan`).
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
