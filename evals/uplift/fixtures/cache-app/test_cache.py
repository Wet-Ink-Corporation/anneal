import unittest

from cache import Cache


class CacheTest(unittest.TestCase):
    def test_a_stored_value_is_returned(self):
        cache = Cache(capacity=2)
        cache.put("a", 1)
        self.assertEqual(cache.get("a"), 1)

    def test_a_missing_key_returns_the_default(self):
        cache = Cache(capacity=2)
        self.assertEqual(cache.get("missing", "none"), "none")

    def test_a_capacity_below_one_is_refused(self):
        with self.assertRaises(ValueError):
            Cache(capacity=0)


if __name__ == "__main__":
    unittest.main()
