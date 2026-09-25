#!/usr/bin/env python3
"""Fails when a tracked file names anything on the hashed denylist.

The denylist holds `<length>:<sha256>` of each private name, normalized to
ASCII [a-z0-9], so the names themselves are never published. A hit prints
only the file and line: CI logs on a public repository are public too.

    python3 scripts/leak_check.py            # check every tracked file
    python3 scripts/leak_check.py --add      # append names read from stdin
"""
import hashlib
import os
import subprocess
import sys
from pathlib import Path

DENYLIST = Path(".github/leak-denylist.sha256")
ALNUM = frozenset(b"abcdefghijklmnopqrstuvwxyz0123456789")


def normalize(data: bytes) -> tuple[bytes, list[int]]:
    """Lowercased [a-z0-9] bytes, and the source line of each kept byte."""
    kept, lines, line = bytearray(), [], 1
    for byte in data.lower():
        if byte == 0x0A:
            line += 1
        elif byte in ALNUM:
            kept.append(byte)
            lines.append(line)
    return bytes(kept), lines


def entry(name: bytes) -> str | None:
    norm, _ = normalize(name)
    return f"{len(norm)}:{hashlib.sha256(norm).hexdigest()}" if norm else None


def load(text: str) -> dict[int, set[str]]:
    by_length: dict[int, set[str]] = {}
    for raw in text.splitlines():
        raw = raw.strip()
        if raw and not raw.startswith("#"):
            length, digest = raw.split(":", 1)
            by_length.setdefault(int(length), set()).add(digest)
    return by_length


def hits(data: bytes, by_length: dict[int, set[str]]) -> list[int]:
    if b"\0" in data:
        return []
    norm, lines = normalize(data)
    found = set()
    for length, digests in by_length.items():
        for start in range(len(norm) - length + 1):
            if hashlib.sha256(norm[start : start + length]).hexdigest() in digests:
                found.add(lines[start])
    return sorted(found)


def check() -> int:
    by_length = load(DENYLIST.read_text())
    if not by_length:
        print(f"leak-check: {DENYLIST} has no entries; nothing was checked", file=sys.stderr)
    tracked = subprocess.run(["git", "ls-files", "-z"], capture_output=True, check=True).stdout
    failed = False
    for name in filter(None, tracked.decode().split("\0")):
        path = Path(name)
        if path == DENYLIST or not path.is_file():
            continue
        for line in hits(path.read_bytes(), by_length):
            print(f"{name}:{line}: names a private project; rephrase this line", file=sys.stderr)
            failed = True
    print("leak-check: private names found" if failed else "leak-check: clean")
    return 1 if failed else 0


def add() -> int:
    present = set(DENYLIST.read_text().splitlines())
    fresh = dict.fromkeys(entry(n.encode()) for n in sys.stdin.read().splitlines())
    new = [e for e in fresh if e and e not in present]
    with DENYLIST.open("a") as out:
        out.writelines(f"{e}\n" for e in new)
    print(f"leak-check: added {len(new)} entries")
    return 0


if __name__ == "__main__":
    root = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True)
    os.chdir(root.stdout.strip())
    sys.exit(add() if sys.argv[1:] == ["--add"] else check())
