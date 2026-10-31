"""docs_check.crate_pins against a copy of this repository's manifests: they
pass as they stand, and each way a sibling can be left unpinned fails."""

import shutil
import tempfile
import unittest
from pathlib import Path

import docs_check

REPO = Path(__file__).resolve().parent.parent


class CratePins(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        for manifest in [REPO / "Cargo.toml", *REPO.glob("crates/*/Cargo.toml")]:
            rel = manifest.relative_to(REPO)
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(manifest, self.root / rel)
        self.version = docs_check.cargo_version(self.root)

    def edit(self, rel, old, new):
        path = self.root / rel
        text = path.read_text()
        self.assertIn(old, text, f"{rel} no longer holds {old!r}")
        path.write_text(text.replace(old, new, 1))

    def problems(self):
        return docs_check.crate_pins(self.root, self.version)

    def test_the_manifests_as_they_stand_pass(self):
        self.assertEqual(self.problems(), [])

    def test_every_crate_is_checked(self):
        self.assertEqual(len(list(self.root.glob("crates/*/Cargo.toml"))), 6)

    def test_a_caret_requirement_fails(self):
        self.edit(
            "crates/fairlead/Cargo.toml",
            f'fairlead-core = {{ version = "={self.version}"',
            f'fairlead-core = {{ version = "{self.version}"',
        )
        [p] = self.problems()
        self.assertIn("crates/fairlead/Cargo.toml: fairlead-core", p)
        self.assertIn(f'"{self.version}", not "={self.version}"', p)

    def test_a_path_only_dependency_fails(self):
        self.edit(
            "crates/fairlead-lang/Cargo.toml",
            f'fairlead-core = {{ version = "={self.version}", path',
            "fairlead-core = { path",
        )
        [p] = self.problems()
        self.assertIn('fairlead-core is required as "None"', p)

    def test_a_version_bump_that_leaves_the_pins_fails(self):
        self.version = "99.0.0"
        problems = self.problems()
        self.assertTrue(problems)
        self.assertTrue(all('not "=99.0.0"' in p for p in problems))

    def test_a_target_specific_dev_dependency_is_checked_too(self):
        path = self.root / "crates/fairlead-guard/Cargo.toml"
        path.write_text(
            path.read_text()
            + '\n[target.\'cfg(unix)\'.dev-dependencies]\nfairlead-tests = { version = "0.1", path = "../fairlead-tests" }\n'
        )
        [p] = self.problems()
        self.assertIn("crates/fairlead-guard/Cargo.toml: fairlead-tests", p)

    def test_other_dependencies_are_left_alone(self):
        self.edit("crates/fairlead/Cargo.toml", 'clap = { version = "4"', 'clap = { version = "^4"')
        self.assertEqual(self.problems(), [])


if __name__ == "__main__":
    unittest.main()
