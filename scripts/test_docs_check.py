"""docs_check against a copy of this repository's docs: it passes as they
stand, and each kind of lag it exists for makes it fail."""

import shutil
import tempfile
import unittest
from pathlib import Path

import docs_check

REPO = Path(__file__).resolve().parent.parent
FILES = [
    "Cargo.toml",
    "CHANGELOG.md",
    "README.md",
    "site/index.html",
    "crates/fairlead/src/main.rs",
]


class DocsCheck(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        for rel in FILES:
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(REPO / rel, self.root / rel)
        shutil.copytree(REPO / "docs/src", self.root / "docs/src")
        self.version = docs_check.cargo_version(self.root)

    def edit(self, rel, old, new):
        path = self.root / rel
        text = path.read_text()
        self.assertIn(old, text, f"{rel} no longer holds {old!r}")
        path.write_text(text.replace(old, new, 1))

    def problems(self, release=None):
        return docs_check.check(self.root, release)

    def released(self):
        """The copy as its release would have it: no Unreleased notes or
        labels left."""
        path = self.root / "CHANGELOG.md"
        text = path.read_text()
        start = text.find("## Unreleased")
        if start >= 0:
            end = text.index("\n## ", start + 1) + 1
            path.write_text(text[:start] + text[end:])
        for page in [self.root / "README.md", *(self.root / "docs/src").glob("*.md")]:
            text = page.read_text()
            page.write_text(text.replace("*Unreleased:*", f"*Since {self.version}:*"))
        site = self.root / "site/index.html"
        site.write_text(site.read_text().replace("unreleased:", f"since {self.version}:"))
        return f"v{self.version}"

    def test_the_docs_as_they_stand_pass(self):
        self.assertEqual(self.problems(), [])

    def test_the_docs_are_ready_for_their_release_once_its_notes_are_dated(self):
        self.assertEqual(self.problems(self.released()), [])

    def test_a_page_documenting_another_release_fails(self):
        self.edit("site/index.html", f"documents Fairlead {self.version}", "documents Fairlead 0.1.0")
        [p] = self.problems()
        self.assertIn("site/index.html", p)
        self.assertIn("documents Fairlead 0.1.0", p)

    def test_promising_a_milestone_the_roadmap_says_shipped_fails(self):
        self.edit("README.md", "*Coming in K4*", "*Coming in K2*")
        [p] = self.problems()
        self.assertIn('"coming in K2", but the roadmap says Shipped in', p)

    def test_a_feature_labelled_with_an_unreleased_version_fails(self):
        self.edit("README.md", "*Since 0.4.0:*", "*Since 9.0.0:*")
        [p] = self.problems()
        self.assertIn('"since 9.0.0" names a version that isn\'t released', p)

    def test_a_today_label_fails_because_it_names_no_version(self):
        self.edit("README.md", "*Since 0.2.0:* owner rules", "*Today:* owner rules")
        [p] = self.problems()
        self.assertIn('"today" says nothing about the version', p)

    def test_a_command_the_readme_leaves_out_fails(self):
        self.edit("README.md", "`coverage import`", "coverage maps")
        [p] = self.problems()
        self.assertIn("doesn't list the `coverage` command", p)

    def test_a_book_page_missing_from_the_contents_fails(self):
        (self.root / "docs/src/orphan.md").write_text("# Orphan\n")
        [p] = self.problems()
        self.assertIn("orphan.md isn't in the book's contents", p)

    def test_releasing_with_unreleased_notes_fails(self):
        tag = self.released()
        self.edit("CHANGELOG.md", f"## {self.version} (", f"## Unreleased\n\n## {self.version} (")
        self.assertEqual(self.problems(), [])
        problems = self.problems(tag)
        self.assertTrue(any("an Unreleased section is left" in p for p in problems), problems)

    def test_an_unreleased_label_passes_a_change_but_not_a_release(self):
        tag = self.released()
        self.edit("README.md", "*Since 0.4.0:*", "*Unreleased:*")
        self.assertEqual(self.problems(), [])
        [p] = self.problems(tag)
        self.assertIn('an "unreleased" label is left', p)

    def test_releasing_a_tag_the_version_doesnt_match_fails(self):
        self.released()
        [p] = self.problems("v9.9.9")
        self.assertIn(f"the tag is v9.9.9, but Cargo.toml says {self.version}", p)

    def test_releasing_with_an_old_action_pin_fails(self):
        tag = self.released()
        self.edit("docs/src/ci.md", f"mahkassem/fairlead@v{self.version}", "mahkassem/fairlead@v0.5.0")
        self.assertEqual(self.problems(), [])
        [p] = self.problems(tag)
        self.assertIn("pinned at v0.5.0", p)


if __name__ == "__main__":
    unittest.main()
