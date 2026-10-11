# `anneal calibrate` fixture

`decisions/` is an anneal home's decision log: `decisions.1.jsonl` (rotated, older) and
`decisions.jsonl` (live). `tests/it/calibrate.rs` copies it into a temporary anneal home.

D1 (noul), worked by hand:

| id | p | action | label (last wins) | o | (p - o)^2 |
|---|---|---|---|---|---|
| -1-0 | 0.1 | defer | `yes`, then `no` | 0 | 0.01 |
| -1-1 | 0.3 | defer | `yes` | 1 | 0.49 |
| -1-2 | 0.8 | ask | `true` | 1 | 0.04 |
| -1-3 | 0.95 | deny | `0` | 0 | 0.9025 |
| -1-4 | 1.0 | deny | `1` | 1 | 0 |
| -1-5 | 0.5 | ask | `maybe` (unusable) | | |
| -1-6 | none (timed out) | | | | |

- n 6 answered, 5 labelled, 1 unanswered, 1 unusable label; asks 2 of 6 (33%).
- Brier: 1.4425 / 5 = 0.2885.
- Bins (count, mean p, observed frequency): [0.1, 0.2) 1, 0.1, 0; [0.3, 0.4) 1, 0.3, 1;
  [0.8, 0.9) 1, 0.8, 1; [0.9, 1.0] 2, 0.975, 0.5; the rest empty.
- Expected cost (a false accept, a positive below the threshold, costs 5; a needless ask 1):
  at the current 0.4, 0.3 is let through (5) and 0.95 is a needless ask (1): 6 / 5 = 1.2.
  Candidates below the next band (0.9): 0.1 costs 2, 0.3 costs 1, 0.8 costs 6. Proposed: 0.3,
  1 / 5 = 0.2.
- Verdict: stay-shadow (too few labelled, Brier above 0.15, ask rate at or above 20%).
- One label (`d1760000000000-9-9`) matches no decision; the torn last line is a skipped row.
