#!/usr/bin/env python3
"""Fail when the README, the site or the book lags the version they describe.

Every pull request runs it as it stands. The release workflow runs it with
`--release <tag>` before anything is built, so a release can't be cut while
the docs describe an older one or still promise what has shipped.

    python3 scripts/docs_check.py                 # the checks every change keeps
    python3 scripts/docs_check.py --release v0.6.0  # also what the release needs
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

VERSION = r"\d+\.\d+\.\d+"
# The pages that say which release they document, in visible text.
MARKED = ["README.md", "docs/src/introduction.md", "site/index.html"]
# The pages whose feature labels say when each feature shipped.
LABELLED = ["README.md", "site/index.html"]


def parse(v: str) -> tuple[int, ...]:
    return tuple(int(p) for p in v.split("."))


def cargo_version(root: Path) -> str:
    text = (root / "Cargo.toml").read_text()
    m = re.search(rf'^\[workspace\.package\][^\[]*?^version = "({VERSION})"', text, re.M | re.S)
    if not m:
        m = re.search(rf'^version = "({VERSION})"', text, re.M)
    if not m:
        raise SystemExit("docs-check: no version in Cargo.toml")
    return m.group(1)


def released(root: Path) -> tuple[list[str], str | None]:
    """The versions the changelog has sections for, and its first heading."""
    text = (root / "CHANGELOG.md").read_text()
    heads = re.findall(r"^## (.+)$", text, re.M)
    versions = [m.group(1) for h in heads if (m := re.match(rf"({VERSION}) \(", h))]
    return versions, heads[0] if heads else None


def commands(root: Path) -> list[str]:
    """The top-level subcommands, from the clap enum, in kebab case."""
    text = (root / "crates/fairlead/src/main.rs").read_text()
    body = text[text.index("enum Command {") :]
    body = body[: body.index("\n}\n")]
    names = re.findall(r"^    ([A-Z][A-Za-z]*)\b", body, re.M)
    return [re.sub(r"(?<!^)([A-Z])", r"-\1", n).lower() for n in names]


def roadmap(root: Path) -> dict[str, str]:
    rows = {}
    for line in (root / "docs/src/roadmap.md").read_text().splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) >= 2 and re.fullmatch(r"K\d+", cells[0]):
            rows[cells[0]] = cells[2] if len(cells) >= 3 else ""
    return rows


def lines_matching(text: str, pattern: str, flags: int = 0):
    for n, line in enumerate(text.splitlines(), 1):
        for m in re.finditer(pattern, line, flags):
            yield n, m


def section(text: str, heading: str) -> str:
    start = text.find(heading)
    if start < 0:
        return ""
    rest = text[start + len(heading) :]
    end = rest.find("\n## ")
    return rest if end < 0 else rest[:end]


def check(root: Path, release: str | None = None) -> list[str]:
    problems: list[str] = []
    version = cargo_version(root)
    versions, first = released(root)
    milestones = roadmap(root)

    for rel in MARKED:
        text = (root / rel).read_text()
        found = list(lines_matching(text, rf"documents Fairlead ({VERSION})", re.I))
        if not found:
            problems.append(f"{rel}: doesn't say which release it documents (\"documents Fairlead {version}\")")
        for n, m in found:
            if m.group(1) != version:
                problems.append(f"{rel}:{n}: documents Fairlead {m.group(1)}, but this is {version}")

    for rel in LABELLED:
        text = (root / rel).read_text()
        for n, m in lines_matching(text, rf"\bsince ({VERSION})", re.I):
            v = m.group(1)
            if v not in versions or parse(v) > parse(version):
                problems.append(f"{rel}:{n}: \"since {v}\" names a version that isn't released")
        for n, m in lines_matching(text, r"\bcoming in (K\d+)", re.I):
            status = milestones.get(m.group(1))
            if status is None:
                problems.append(f"{rel}:{n}: \"coming in {m.group(1)}\" names no roadmap milestone")
            elif status.startswith("Shipped"):
                problems.append(f"{rel}:{n}: \"coming in {m.group(1)}\", but the roadmap says {status}")
        for n, m in lines_matching(text, r"\*Today:\*|>today:", re.I):
            problems.append(f"{rel}:{n}: \"today\" says nothing about the version; say which release it shipped in")

    for k, status in milestones.items():
        m = re.match(rf"(?:Partly shipped|Shipped) in ({VERSION})\b|Planned$|In progress\b", status)
        if not m:
            problems.append(f"docs/src/roadmap.md: {k}'s status \"{status}\" isn't Shipped in X.Y.Z, Partly shipped in X.Y.Z, In progress or Planned")
        elif m.group(1) and (m.group(1) not in versions or parse(m.group(1)) > parse(version)):
            problems.append(f"docs/src/roadmap.md: {k} says shipped in {m.group(1)}, which isn't released")

    readme = (root / "README.md").read_text()
    does = section(readme, "## What it does\n")
    book = "\n".join(p.read_text() for p in sorted((root / "docs/src").glob("*.md")))
    for name in commands(root):
        if name == "help":
            continue
        if not re.search(rf"`{re.escape(name)}(\s[^`]*)?`", does):
            problems.append(f"README.md: \"What it does\" doesn't list the `{name}` command")
        if not re.search(rf"fairlead {re.escape(name)}\b", book):
            problems.append(f"docs/src: no page shows `fairlead {name}`")

    summary = (root / "docs/src/SUMMARY.md").read_text()
    for page in sorted((root / "docs/src").glob("*.md")):
        if page.name != "SUMMARY.md" and f"({page.name})" not in summary:
            problems.append(f"docs/src/SUMMARY.md: {page.name} isn't in the book's contents")

    if first and re.match(VERSION, first) and not first.startswith(f"{version} ("):
        problems.append(f"CHANGELOG.md: the top section is {first}, but Cargo.toml says {version}")

    if release is not None:
        problems += check_release(root, release, version, first)
    return problems


def check_release(root: Path, tag: str, version: str, first: str | None) -> list[str]:
    problems = []
    if tag != f"v{version}":
        problems.append(f"the tag is {tag}, but Cargo.toml says {version}")
    if not first or not first.startswith(f"{version} ("):
        problems.append(f"CHANGELOG.md: the top section is {first}, not {version}; date the release's notes")
    changelog = (root / "CHANGELOG.md").read_text()
    if re.search(r"^## Unreleased", changelog, re.M):
        problems.append("CHANGELOG.md: an Unreleased section is left")
    readme = (root / "README.md").read_text()
    m = re.search(rf"latest release, v({VERSION})", readme)
    if not m or m.group(1) != version:
        problems.append(f"README.md: the Status section doesn't name v{version} as the latest release")
    pages = [root / "README.md", root / "site/index.html", *sorted((root / "docs/src").glob("*.md"))]
    for page in pages:
        text = page.read_text()
        rel = page.relative_to(root)
        for n, m in lines_matching(text, rf"mahkassem/fairlead@v({VERSION})"):
            if m.group(1) != version:
                problems.append(f"{rel}:{n}: the action is pinned at v{m.group(1)}, not v{version}")
        for n, _ in lines_matching(text, r"\bunreleased:", re.I):
            problems.append(f"{rel}:{n}: an \"unreleased\" label is left")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--release", metavar="TAG", help="also check what releasing TAG needs")
    ap.add_argument("--root", default=Path(__file__).resolve().parent.parent, type=Path)
    args = ap.parse_args()
    release = args.release
    if release is None:
        ref = os.environ.get("GITHUB_REF", "")
        if ref.startswith("refs/tags/v"):
            release = ref.removeprefix("refs/tags/")
    problems = check(args.root, release)
    for p in problems:
        print(f"docs-check: {p}")
    if problems:
        print(f"docs-check: {len(problems)} problem(s); the docs must describe the release they ship with")
        return 1
    what = f"ready for {release}" if release else "current"
    print(f"docs-check: the README, the site and the book are {what}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
