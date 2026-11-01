import json
import os
import tempfile
import unittest

from tickets import GraphError, load


def graph_file(tickets) -> str:
    handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
    with handle:
        json.dump(tickets, handle)
    return handle.name


class LoadTest(unittest.TestCase):
    def test_tickets_load_in_file_order(self):
        path = graph_file([{"id": "T1"}, {"id": "T2", "blocked_by": ["T1"]}])
        self.addCleanup(os.remove, path)
        self.assertEqual([t.id for t in load(path)], ["T1", "T2"])

    def test_a_file_that_is_not_an_array_is_refused(self):
        path = graph_file({"id": "T1"})
        self.addCleanup(os.remove, path)
        with self.assertRaises(GraphError):
            load(path)


if __name__ == "__main__":
    unittest.main()
