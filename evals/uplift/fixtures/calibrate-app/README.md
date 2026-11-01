# calibrate

Picks the threshold at which a decision point acts on a model's probability. `labels.csv` is the labelled set: one decision per row, the model's `probability` and the human's `label` (1: the point should have acted, 0: it should not).

- `calibrate.cost(threshold, rows)` is the expected cost of a threshold on the labelled set: one for each false positive (probability at or above the threshold, label 0) and one for each false negative (below it, label 1).
- `calibrate.propose(current, rows)` returns the threshold to use. It keeps the current threshold unless a candidate costs strictly less: a candidate that only equals it is no improvement.
- `python3 calibrate.py labels.csv --current 0.60` prints the proposal.

The tests use the standard library's `unittest` only: run them with `python3 -m unittest`. `pytest` is not installed.
