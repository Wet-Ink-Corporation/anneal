import unittest

from calibrate import cost, propose


class CostTest(unittest.TestCase):
    def test_false_positives_and_false_negatives_each_cost_one(self):
        rows = [(0.7, False), (0.3, True), (0.9, True), (0.1, False)]
        self.assertEqual(cost(0.5, rows), 2)


class ProposeTest(unittest.TestCase):
    def test_a_cheaper_candidate_replaces_the_current_threshold(self):
        rows = [(0.52, False), (0.58, True), (0.9, True), (0.1, False)]
        self.assertEqual(propose(0.50, rows, candidates=(0.50, 0.55)), 0.55)

    def test_a_dearer_candidate_does_not(self):
        rows = [(0.52, True), (0.9, True), (0.1, False)]
        self.assertEqual(propose(0.50, rows, candidates=(0.50, 0.55)), 0.50)


if __name__ == "__main__":
    unittest.main()
