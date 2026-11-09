# Clef calibration report, November 2026 (#44)

This is the gate report for story #44. Ryan decided on 2026-11-08 to cut it from the labels that exist, without a v0.1.0 tag or release (Bench call E, `AGENTS.md` decisions log). The bar a point must clear to leave shadow is in `docs/calibrate.md` and `docs/release-0.1.0.md`: n ≥ 50 labelled decisions, Brier ≤ 0.15, and an ask rate under 20%.

## The run

Ryan ran `cargo run --release -p anneal-cli -- calibrate` on 2026-11-09, from a checkout of `main`, against his anneal home on Windows. The envelope's `log` named `C:\Users\ryanm\.anneal\decisions\decisions.jsonl`, and no log file existed there (`files` lists only files that exist, so neither the live log nor a rotated one was found), so nothing was read. The envelope reported `files: []`, `skipped_rows: 0`, `unmatched_labels: 0`. Every point reported mode `off` and the verdict `stay-shadow` (`too_few_labelled`), and nothing was applied.

| Point | Mode | n | Labelled | Brier | Ask rate | Verdict |
|---|---|---|---|---|---|---|
| D1 guard | off | 0 | 0 | none | none | stays in shadow (n = 0) |
| D2 orchestrate | off | 0 | 0 | none | none | stays in shadow (n = 0) |
| D3 route | off | 0 | 0 | none | none | stays in shadow (n = 0) |
| D4 completeness | off | 0 | 0 | none | none | stays in shadow (n = 0) |
| D5 verify | off | 0 | 0 | none | none | stays in shadow (n = 0) |
| D6 ruled-before | off | 0 | 0 | none | none | stays in shadow (n = 0) |
| D7 grill | off | 0 | 0 | none | none | stays in shadow (n = 0) |

## What it means

Clef is off in that anneal home (mode `off`: no account configured) and no decision log exists there, so no decision was recorded and none could be labelled. That says nothing about how well Clef's probabilities are calibrated: it is the absence of evidence, not a failed calibration.

- No point is promoted. All seven ship in shadow, as the built-in thresholds table (`anneal playbook show clef-thresholds`) sets them, and no threshold changes.
- Calibration moves with the code. In redkiln the scoring goes to `redkiln-calibrate` (`docs/redkiln-move.md`); the first labelled run belongs there, after Bench turns Clef on in shadow and labels decisions with `decide label`.
- The automatic label sources the blueprint names (for D1, the human's answer when asked, or a revert within 24 h) still do not exist (`docs/calibrate.md`). Until they land, every label is typed by hand, so reaching n ≥ 50 per point needs deliberate labelling.

## Proves and does not prove

This proves the calibrate command runs on a real Windows anneal home and reports a missing log as zero decisions without failing. It does not prove anything about Brier scores, bins or ask rates for any point, because none were measured.
