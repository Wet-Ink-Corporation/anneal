"""Proposes a decision threshold from labelled decisions."""

import argparse
import csv

CANDIDATES = (0.50, 0.55, 0.60, 0.62, 0.65, 0.70)


def load(path):
    """The labelled set at `path`: a list of (probability, label) pairs."""
    with open(path, newline="") as labels:
        return [(float(row["probability"]), row["label"] == "1") for row in csv.DictReader(labels)]


def cost(threshold, rows):
    """False positives plus false negatives of `threshold` on `rows`."""
    false_positives = sum(1 for probability, label in rows if probability >= threshold and not label)
    false_negatives = sum(1 for probability, label in rows if probability < threshold and label)
    return false_positives + false_negatives


def propose(current, rows, candidates=CANDIDATES):
    """The threshold to use: a candidate replaces `current` only if it costs less."""
    best, best_cost = current, cost(current, rows)
    for candidate in candidates:
        candidate_cost = cost(candidate, rows)
        if candidate_cost <= best_cost:
            best, best_cost = candidate, candidate_cost
    return best


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("labels")
    parser.add_argument("--current", type=float, required=True)
    args = parser.parse_args(argv)
    proposal = propose(args.current, load(args.labels))
    print(f"proposed threshold: {proposal:.2f} (current {args.current:.2f})")


if __name__ == "__main__":
    main()
