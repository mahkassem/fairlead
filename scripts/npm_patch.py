#!/usr/bin/env python3
"""Puts Fairlead's installer into the npm package dist generates.

dist's `binary-install.js` downloads the binary on first use into a folder
it empties first, so two first runs at once can spawn a half-written binary
(ETXTBSY, #211). `npm/binary-install.js` extracts beside that folder and
renames it into place. It is a port of one dist release's file: the package
must hold exactly that file, or the port is stale and publishing stops.

    python3 scripts/npm_patch.py npm/*-npm-package.tar.gz   # before npm publish
    python3 scripts/npm_patch.py --check                    # dist is still the version ported
"""
import hashlib
import io
import re
import sys
import tarfile
from pathlib import Path

PORTED_FROM = "0.32.0"
UPSTREAM_SHA256 = "f9cd1e11d9fdbbcaec5cf0a52145a0495d109967ac60363ced9e2bb6f28472dd"
MEMBER = "package/binary-install.js"
OURS = Path(__file__).resolve().parent.parent / "npm" / "binary-install.js"


def dist_version(root: Path) -> str | None:
    text = (root / "dist-workspace.toml").read_text()
    m = re.search(r'^cargo-dist-version\s*=\s*"([^"]+)"', text, re.M)
    return m.group(1) if m else None


def patch(tarball: Path, ours: bytes, upstream: str = UPSTREAM_SHA256) -> None:
    with tarfile.open(tarball, "r:gz") as src:
        members = [(m, src.extractfile(m).read() if m.isfile() else None) for m in src.getmembers()]
    names = [m.name for m, _ in members]
    if MEMBER not in names:
        raise SystemExit(f"npm-patch: {tarball} has no {MEMBER}")
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w:gz") as dst:
        for member, data in members:
            if member.name == MEMBER:
                found = hashlib.sha256(data).hexdigest()
                if found != upstream:
                    raise SystemExit(
                        f"npm-patch: {tarball}'s {MEMBER} isn't the dist {PORTED_FROM} file "
                        f"npm/binary-install.js was ported from (sha256 {found}); port it again"
                    )
                data = ours
                member.size = len(data)
            dst.addfile(member, io.BytesIO(data) if data is not None else None)
    tarball.write_bytes(out.getvalue())


def main(argv: list[str]) -> int:
    root = Path(__file__).resolve().parent.parent
    if argv == ["--check"]:
        version = dist_version(root)
        if version != PORTED_FROM:
            print(
                f"npm-patch: dist is {version}, but npm/binary-install.js is ported from "
                f"{PORTED_FROM}; port it to the new dist's file and update scripts/npm_patch.py",
                file=sys.stderr,
            )
            return 1
        print(f"npm-patch: npm/binary-install.js matches dist {PORTED_FROM}")
        return 0
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    ours = OURS.read_bytes()
    for name in argv:
        patch(Path(name), ours)
        print(f"npm-patch: {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
