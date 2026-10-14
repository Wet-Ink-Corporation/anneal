---
name: build
description: Use when build000 build001 build002 build003 build004 build005 build006 build007 build008 build009 build010 build011 build012 build013 build014 build015 build016 build017 build018 build019 build020 build021 build022 build0, even if it looks done. Do NOT use for other work (use `diagnose`).
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
