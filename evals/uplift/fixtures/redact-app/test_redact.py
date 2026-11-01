import unittest

from redact import redact


class RedactTest(unittest.TestCase):
    def test_a_token_assignment_is_hidden(self):
        self.assertEqual(redact("API_TOKEN=abc123 deploy"), "'API_TOKEN=***' deploy")

    def test_a_mysql_password_is_hidden(self):
        self.assertEqual(redact("mysql -p hunter2 db"), "mysql -p '***' db")

    def test_a_command_without_secrets_is_unchanged(self):
        self.assertEqual(redact("ls -la"), "ls -la")


if __name__ == "__main__":
    unittest.main()
