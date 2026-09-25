"""Proves the leak check matches through separators and case, skips binary
files, reports lines rather than the matched text, and reads every text field
of a GitHub event."""
import json
import os
import tempfile
import unittest
from unittest import mock

from leak_check import entry, event_text, hits, load

ANY_NAME = b"example-project"
DENY = load(entry(ANY_NAME) + "\n")


class LeakCheck(unittest.TestCase):
    def test_finds_the_name_on_its_line(self):
        self.assertEqual(hits(b"first\nsee example-project here\n", DENY), [2])

    def test_finds_it_through_case_and_separators(self):
        self.assertEqual(hits(b"Example_Project\nEXAMPLE PROJECT\n", DENY), [1, 2])

    def test_finds_it_split_across_lines_on_the_first(self):
        self.assertEqual(hits(b"example\nproject\n", DENY), [1])

    def test_passes_text_without_it(self):
        self.assertEqual(hits(b"an example of a project\n", DENY), [])

    def test_skips_binary_files(self):
        self.assertEqual(hits(b"\0example-project", DENY), [])

    def test_entry_hides_the_name(self):
        self.assertNotIn("example", entry(ANY_NAME))
        self.assertTrue(entry(ANY_NAME).startswith("14:"))

    def test_event_text_reads_every_kind_and_skips_empty_bodies(self):
        event = {
            "issue": {"title": "a title", "body": "an issue body", "number": 1},
            "pull_request": {"title": "a pull request", "body": 0},
            "comment": {"body": "a comment"},
            "review": {"body": None},
        }
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(event, f)
        try:
            with mock.patch.dict(os.environ, {"GITHUB_EVENT_PATH": f.name}):
                found = dict(event_text())
        finally:
            os.unlink(f.name)
        self.assertEqual(
            found,
            {
                "issue title": b"a title",
                "issue body": b"an issue body",
                "pull_request title": b"a pull request",
                "comment body": b"a comment",
            },
        )


if __name__ == "__main__":
    unittest.main()
