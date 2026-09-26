"""Proves the probe keeps failure lines with their context, strips colour
codes and timestamps, and caps each excerpt."""
import unittest

from probe_logs import MAX_EXCERPT_LINES, excerpt


class Excerpt(unittest.TestCase):
    def test_keeps_fail_lines_with_two_after_and_strips_noise(self):
        log = "\n".join(
            [
                "2026-01-01T00:00:00.0000000Z setup",
                "2026-01-01T00:00:01.0000000Z \x1b[31m FAIL \x1b[39m src/a.test.ts > adds",
                "2026-01-01T00:00:01.1000000Z AssertionError: expected 1",
                "2026-01-01T00:00:01.2000000Z   at src/a.test.ts:3:5",
                "2026-01-01T00:00:01.3000000Z unrelated",
            ]
        )
        self.assertEqual(
            excerpt(log),
            [" FAIL  src/a.test.ts > adds", "AssertionError: expected 1", "  at src/a.test.ts:3:5"],
        )

    def test_overlapping_failures_keep_each_line_once(self):
        self.assertEqual(excerpt("FAIL a\nFAIL b\nx\ny\nz"), ["FAIL a", "FAIL b", "x", "y"])

    def test_caps_the_excerpt(self):
        log = "\n".join(f"FAIL test/{i}.test.ts" for i in range(200))
        self.assertEqual(len(excerpt(log)), MAX_EXCERPT_LINES)

    def test_a_log_without_failures_gives_nothing(self):
        self.assertEqual(excerpt("all good\npassed"), [])


if __name__ == "__main__":
    unittest.main()
