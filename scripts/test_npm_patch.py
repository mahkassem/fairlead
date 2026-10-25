"""Proves the npm patch swaps only the installer's files, keeps every other
entry and its mode, refuses a package whose files aren't the ones ported, that
the ports it ships are the files in npm/, and that dist is still the version
the ports came from."""
import hashlib
import io
import tarfile
import tempfile
import unittest
from pathlib import Path

from npm_patch import PORTED_FROM, PORTS, dist_version, ours, patch

INSTALL = "package/binary-install.js"
SELECT = "package/binary.js"
UPSTREAM = {INSTALL: b"// dist's installer\n", SELECT: b"// dist's platform pick\n"}
SUMS = {name: hashlib.sha256(data).hexdigest() for name, data in UPSTREAM.items()}
OURS = {INSTALL: b"// ours\n", SELECT: b"// our pick\n"}


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
            INSTALL: (UPSTREAM[INSTALL], 0o644),
            SELECT: (UPSTREAM[SELECT], 0o644),
            "package/run-fairlead.js": (b"#!/usr/bin/env node\n", 0o755),
        })
        patch(path, OURS, SUMS)
        self.assertEqual(entries(path), {
            "package/package.json": (b"{}\n", 0o644),
            INSTALL: (OURS[INSTALL], 0o644),
            SELECT: (OURS[SELECT], 0o644),
            "package/run-fairlead.js": (b"#!/usr/bin/env node\n", 0o755),
        })

    def test_a_file_dist_changed_stops_the_publish(self):
        for changed in UPSTREAM:
            files = {name: (data, 0o644) for name, data in UPSTREAM.items()}
            files[changed] = (b"// a newer dist's file\n", 0o644)
            path = package(files)
            before = path.read_bytes()
            with self.assertRaises(SystemExit) as stop:
                patch(path, OURS, SUMS)
            self.assertIn("port it again", str(stop.exception))
            self.assertIn(changed, str(stop.exception))
            self.assertEqual(path.read_bytes(), before)

    def test_a_package_missing_a_ported_file_stops_the_publish(self):
        for kept in UPSTREAM:
            path = package({kept: (UPSTREAM[kept], 0o644)})
            with self.assertRaises(SystemExit):
                patch(path, OURS, SUMS)

    def test_the_ports_are_the_files_in_npm(self):
        npm = Path(__file__).resolve().parent.parent / "npm"
        replace = ours()
        self.assertEqual(sorted(replace), sorted(PORTS))
        for name, data in replace.items():
            self.assertEqual(data, (npm / Path(name).name).read_bytes())

    def test_dist_is_the_version_the_installer_was_ported_from(self):
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(dist_version(root), PORTED_FROM)


if __name__ == "__main__":
    unittest.main()
