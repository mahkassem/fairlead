#!/usr/bin/env python3
"""Fails when tracked files or public text name anything on the hashed denylist.

The denylist holds `<length>:<sha256>` of each private name, normalized to
ASCII [a-z0-9], so the names themselves are never published. A hit prints
only where it is: CI logs on a public repository are public too.

    python3 scripts/leak_check.py                     # every tracked file
    python3 scripts/leak_check.py --text < draft.md   # text before posting it
    python3 scripts/leak_check.py --event             # the GitHub event's text
    python3 scripts/leak_check.py --commits BASE HEAD # commit messages
    python3 scripts/leak_check.py --add               # append names from stdin
"""
import json
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


def report(sources: list[tuple[str, bytes]]) -> int:
    by_length = load(DENYLIST.read_text())
    if not by_length:
        print(f"leak-check: {DENYLIST} has no entries; nothing was checked", file=sys.stderr)
    failed = False
    for where, data in sources:
        for line in hits(data, by_length):
            print(f"{where}:{line}: names a private project; rephrase this line", file=sys.stderr)
            failed = True
    print("leak-check: private names found" if failed else "leak-check: clean")
    return 1 if failed else 0


def tracked_files() -> list[tuple[str, bytes]]:
    listed = subprocess.run(["git", "ls-files", "-z"], capture_output=True, check=True).stdout
    paths = (Path(n) for n in filter(None, listed.decode().split("\0")))
    return [(str(p), p.read_bytes()) for p in paths if p != DENYLIST and p.is_file()]


def event_text() -> list[tuple[str, bytes]]:
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    kinds = ("issue", "pull_request", "comment", "review")
    if not any(isinstance(event.get(kind), dict) for kind in kinds):
        print("leak-check: the event carries no issue, pull request, comment or review", file=sys.stderr)
    fields = [(kind, part) for kind in kinds for part in ("title", "body")]
    return [
        (f"{kind} {part}", str(event[kind][part]).encode())
        for kind, part in fields
        if isinstance(event.get(kind), dict) and event[kind].get(part)
    ]


def commit_messages(base: str, head: str) -> list[tuple[str, bytes]]:
    shas = subprocess.run(["git", "rev-list", f"{base}..{head}"], capture_output=True, text=True, check=True)
    return [
        (f"commit {sha[:7]}", subprocess.run(["git", "log", "-1", "--format=%B", sha], capture_output=True, check=True).stdout)
        for sha in shas.stdout.split()
    ]


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
    args = sys.argv[1:]
    if args == ["--add"]:
        sys.exit(add())
    if args == ["--text"]:
        sys.exit(report([("text", sys.stdin.buffer.read())]))
    if args == ["--event"]:
        sys.exit(report(event_text()))
    if len(args) == 3 and args[0] == "--commits":
        sys.exit(report(commit_messages(args[1], args[2])))
    sys.exit(report(tracked_files()))
