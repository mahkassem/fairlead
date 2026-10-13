"""Proves the npm patch swaps only the installer, keeps every other entry and
its mode, refuses a package whose installer isn't the one ported, and that
dist is still the version the port came from."""
import hashlib
import io
import tarfile
import tempfile
import unittest
from pathlib import Path

from npm_patch import MEMBER, PORTED_FROM, dist_version, patch

UPSTREAM = b"// dist's installer\n"


def package(files: dict[str, tuple[bytes, int]]) -> Path:
    path = Path(tempfile.mkdtemp()) / "fairlead-npm-package.tar.gz"
    with tarfile.open(path, "w:gz") as tar:
        for name, (data, mode) in files.items():
            info = tarfile.TarInfo(name)
            info.size, info.mode = len(data), mode
            tar.addfile(info, io.BytesIO(data))
    return path


def entries(path: Path) -> dict[str, tuple[bytes, int]]:
    with tarfile.open(path, "r:gz") as tar:
        return {m.name: (tar.extractfile(m).read(), m.mode) for m in tar.getmembers()}


class NpmPatch(unittest.TestCase):
    def test_swaps_the_installer_and_keeps_the_rest(self):
        path = package({
            "package/package.json": (b"{}\n", 0o644),
            MEMBER: (UPSTREAM, 0o644),
            "package/run-fairlead.js": (b"#!/usr/bin/env node\n", 0o755),
        })
        patch(path, b"// ours\n", hashlib.sha256(UPSTREAM).hexdigest())
        self.assertEqual(entries(path), {
            "package/package.json": (b"{}\n", 0o644),
            MEMBER: (b"// ours\n", 0o644),
            "package/run-fairlead.js": (b"#!/usr/bin/env node\n", 0o755),
        })

    def test_an_installer_dist_changed_stops_the_publish(self):
        path = package({MEMBER: (b"// a newer dist's installer\n", 0o644)})
        before = path.read_bytes()
        with self.assertRaises(SystemExit) as stop:
            patch(path, b"// ours\n", hashlib.sha256(UPSTREAM).hexdigest())
        self.assertIn("port it again", str(stop.exception))
        self.assertEqual(path.read_bytes(), before)

    def test_a_package_without_the_installer_stops_the_publish(self):
        path = package({"package/package.json": (b"{}\n", 0o644)})
        with self.assertRaises(SystemExit):
            patch(path, b"// ours\n", hashlib.sha256(UPSTREAM).hexdigest())

    def test_dist_is_the_version_the_installer_was_ported_from(self):
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(dist_version(root), PORTED_FROM)


if __name__ == "__main__":
    unittest.main()
