import os
import tempfile
import unittest

from snapshot import MAX_BYTES, read_snapshot


class SnapshotTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)

    def write(self, size):
        path = os.path.join(self.dir.name, "snapshot.bin")
        with open(path, "wb") as snapshot:
            snapshot.write(b"x" * size)
        return path

    def test_a_small_snapshot_is_read_whole(self):
        self.assertEqual(read_snapshot(self.write(10)), b"x" * 10)

    def test_a_snapshot_at_the_cap_is_read_whole(self):
        self.assertEqual(len(read_snapshot(self.write(MAX_BYTES))), MAX_BYTES)


if __name__ == "__main__":
    unittest.main()
