#!/usr/bin/env python3
"""Puts Fairlead's installer into the npm package dist generates.

dist's `binary-install.js` downloads the binary on first use into a folder
it empties first, so two first runs at once can spawn a half-written binary
(ETXTBSY, #211). `npm/binary-install.js` extracts beside that folder and
renames it into place. dist's `binary.js` fails the install on a platform
with no build (#218); `npm/binary.js` warns and lets it succeed, and picks
the musl build on a musl system. Each is a port of one dist release's file:
the package must hold exactly those files, or a port is stale and
publishing stops.

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
NPM = Path(__file__).resolve().parent.parent / "npm"
# Each file the package holds that Fairlead replaces, with the sha256 of the
# dist file it was ported from.
PORTS = {
    "package/binary-install.js": "f9cd1e11d9fdbbcaec5cf0a52145a0495d109967ac60363ced9e2bb6f28472dd",
    "package/binary.js": "4355398e7705fdf14d3c7ece43ec1f2fee2010eecc590cc9a311b44d8258544b",
}


def dist_version(root: Path) -> str | None:
    text = (root / "dist-workspace.toml").read_text()
    m = re.search(r'^cargo-dist-version\s*=\s*"([^"]+)"', text, re.M)
    return m.group(1) if m else None


def ours() -> dict[str, bytes]:
    return {name: (NPM / Path(name).name).read_bytes() for name in PORTS}


def patch(tarball: Path, replace: dict[str, bytes], upstream: dict[str, str] = PORTS) -> None:
    """Swaps each member of `upstream` for its bytes in `replace`."""
    with tarfile.open(tarball, "r:gz") as src:
        members = [(m, src.extractfile(m).read() if m.isfile() else None) for m in src.getmembers()]
    names = [m.name for m, _ in members]
    for name in upstream:
        if name not in names:
            raise SystemExit(f"npm-patch: {tarball} has no {name}")
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w:gz") as dst:
        for member, data in members:
            if member.name in upstream:
                found = hashlib.sha256(data).hexdigest()
                if found != upstream[member.name]:
                    raise SystemExit(
                        f"npm-patch: {tarball}'s {member.name} isn't the dist {PORTED_FROM} file "
                        f"npm/{Path(member.name).name} was ported from (sha256 {found}); port it again"
                    )
                data = replace[member.name]
                member.size = len(data)
            dst.addfile(member, io.BytesIO(data) if data is not None else None)
    tarball.write_bytes(out.getvalue())


def main(argv: list[str]) -> int:
    root = Path(__file__).resolve().parent.parent
    if argv == ["--check"]:
        version = dist_version(root)
        if version != PORTED_FROM:
            print(
                f"npm-patch: dist is {version}, but npm/binary-install.js and npm/binary.js are "
                f"ported from {PORTED_FROM}; port them to the new dist's files and update "
                "scripts/npm_patch.py",
                file=sys.stderr,
            )
            return 1
        print(f"npm-patch: npm/binary-install.js and npm/binary.js match dist {PORTED_FROM}")
        return 0
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    replace = ours()
    for name in argv:
        patch(Path(name), replace)
        print(f"npm-patch: {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
