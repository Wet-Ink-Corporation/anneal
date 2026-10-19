import contextlib
import io
import unittest

from app.cli import main


class UsageTest(unittest.TestCase):
    def test_usage_without_a_command(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = main([])
        self.assertEqual(code, 2)
        self.assertIn("usage", out.getvalue())


if __name__ == "__main__":
    unittest.main()
