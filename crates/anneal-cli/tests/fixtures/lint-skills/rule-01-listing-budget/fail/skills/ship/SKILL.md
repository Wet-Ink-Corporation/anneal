---
name: ship
description: Use when ship000 ship001 ship002 ship003 ship004 ship005 ship006 ship007 ship008 ship009 ship010 ship011 ship012 ship013 ship014 ship015 ship016 ship017 ship018 ship019 ship020 ship021 ship022 ship023 ship024 ship025 sh, even if it looks done. Do NOT use for other work (use `orchestrate`).
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
