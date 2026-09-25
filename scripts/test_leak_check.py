"""Proves the leak check matches through separators and case, skips binary
files, and reports lines rather than the matched text."""
import unittest

from leak_check import entry, hits, load

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


if __name__ == "__main__":
    unittest.main()
