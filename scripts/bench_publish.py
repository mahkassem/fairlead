#!/usr/bin/env python3
"""Turns the bench workflow's artifacts into committed files.

For each benchmark config in bench/, copies the grown dataset from
`<artifacts>/bench-<slug>/data.jsonl` into bench/data/, and writes
docs/src/benchmarks.md from each `report.json`. Report text comes from other
repositories' CI (job names, paths), so it is only ever printed inside code
spans with backticks and pipes removed.

    python3 scripts/bench_publish.py ARTIFACTS_DIR [--commit SHA]
"""

import argparse
import json
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent


def plain(text, limit=200):
    """Untrusted text made safe for a Markdown code span or table cell."""
    text = re.sub(r"[`|\r\n<>]", " ", str(text))
    return text[:limit]


def pct(value):
    return "-" if value is None else f"{value * 100:.1f}%"


def secs(value):
    return "-" if value is None else f"{value:.2f} s"


def read_rows(path):
    rows = []
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                rows.append(json.loads(line))
    return rows


def grow_dataset(new, current):
    """Replaces `current` with `new` only when `new` keeps every row of it."""
    new_rows = read_rows(new)
    old_keys = {(r["run_id"], r["attempt"]) for r in read_rows(current)}
    new_keys = {(r["run_id"], r["attempt"]) for r in new_rows}
    if not old_keys <= new_keys:
        return False
    if not new_rows:
        return True
    current.parent.mkdir(parents=True, exist_ok=True)
    current.write_text(new.read_text(encoding="utf-8"), encoding="utf-8")
    return True


def repo_section(slug, report, rows, fetch_line):
    judged = report["hits"] + len(report["misses"])
    gate = judged >= report["min_failures"]
    first = min((r["created_at"][:10] for r in rows), default="-")
    last = max((r["created_at"][:10] for r in rows), default="-")
    lines = [
        f"## {plain(report['repo'])}",
        "",
        f"Window {report['from']} to {report['until']}; dataset `bench/data/{slug}.jsonl`, "
        f"{len(rows)} run attempts from {first} to {last}. Fetch: `{plain(fetch_line)}`.",
        "",
        "| Measure | Value |",
        "| --- | --- |",
        f"| Runs replayed | {report['runs']} |",
        f"| Attributed failures (gate {report['min_failures']}) | {judged} ({'met' if gate else 'not met'}) |",
        f"| Recall | {pct(report['recall'])} |",
    ]
    if report.get("quarantine"):
        lines += [
            f"| Recall with quarantined tests counted (raw) | {pct(report['raw_recall'])} (n={report['raw_judged']}) |",
            f"| Recall with them left out (adjusted) | {pct(report['recall'])} (n={report['judged']}) |",
        ]
    lines += [
        f"| Strict recall (unconfirmed as misses) | {pct(report['strict_recall'])} |",
        f"| Hits: selected, run everything, checks | {report['hits_selected']}, {report['hits_run_all']}, {report['hits_check']} |",
        f"| Misses | {len(report['misses'])} |",
        f"| Flaky, unconfirmed, unattributed | {report['flaky']}, {report['unconfirmed']}, {report['unattributed']} |",
        f"| Unavailable, errors, ignored | {report['unavailable']}, {report['errors']}, {report['ignored']} |",
        f"| Test files selected: median, p90 | {pct(report['median_selected'])}, {pct(report['p90_selected'])} |",
        f"| Plans that selected everything | {pct(report['run_all_share'])} |",
        f"| Plan time: first (cold), median, p90 | {secs(report['first_plan_seconds'])}, "
        f"{secs(report['median_plan_seconds'])}, {secs(report['p90_plan_seconds'])} |",
    ]
    for event, e in sorted(report.get("by_event", {}).items()):
        lines.append(
            f"| Recall on `{plain(event)}` runs (strict) | {pct(e['recall'])} ({pct(e['strict_recall'])}) |"
        )
    if report.get("quarantine"):
        lines += ["", "Quarantined tests (declared in the bench config, applied only while the data bears them out):", ""]
        for q in report["quarantine"]:
            lines.append(
                f"- `{plain(q['path'])}` in jobs `{plain(q['job'])}`: {plain(q['status'])}, "
                f"{q['absorbed']} failures absorbed ({q['would_hit']} would-be hits, {q['would_miss']} would-be misses), "
                f"{q['pulls']} pull requests, until {plain(q['until'])}. {plain(q['reason'], 300)}"
            )
            if q.get("jobs"):
                lines.append(f"  - Absorbed in: {', '.join(f'`{plain(j)}`' for j in q['jobs'])}")
            if q.get("other_jobs"):
                lines.append(f"  - Also failed in: {', '.join(f'`{plain(j)}`' for j in q['other_jobs'])}")
    if report.get("widened_by"):
        lines += ["", "Plans that selected everything, by cause:", ""]
        for why, n in sorted(report["widened_by"].items(), key=lambda kv: (-kv[1], kv[0]))[:10]:
            lines.append(f"- `{plain(why)}`: {n}")
    if report.get("unwatched"):
        lines += ["", "Failed jobs no rule watches:", ""]
        for job, n in sorted(report["unwatched"].items()):
            lines.append(f"- `{plain(job)}`: {n}")
    if report["misses"]:
        lines += ["", "Misses:", ""]
        for m in report["misses"]:
            changed = ", ".join(plain(c, 120) for c in m["changed"][:5])
            lines.append(
                f"- run {m['run_id']} attempt {m['attempt']}: `{plain(m['target'])}`, changed `{changed}`"
            )
    return lines


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("artifacts", type=pathlib.Path)
    parser.add_argument("--commit", default="")
    parser.add_argument("--root", type=pathlib.Path, default=ROOT)
    args = parser.parse_args(argv)
    root = args.root
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    commit = f" at `{plain(args.commit[:12])}`" if args.commit else ""
    out = [
        "# Benchmarks",
        "",
        "Replay results on public repositories, written by the `bench` workflow. "
        "See [Replay](replay.md) for what each measure means; the configs are in `bench/`.",
        "",
        f"Planned with Fairlead {version}{commit}.",
    ]
    for config in sorted((root / "bench").glob("*.toml")):
        slug = config.stem
        folder = args.artifacts / f"bench-{slug}"
        data = root / "bench" / "data" / f"{slug}.jsonl"
        if (folder / "data.jsonl").exists() and not grow_dataset(folder / "data.jsonl", data):
            print(f"{slug}: the new dataset drops recorded rows; kept the old one", file=sys.stderr)
        report_path = folder / "report.json"
        if not report_path.exists():
            out += ["", f"## {slug}", "", "No report in this run."]
            continue
        report = json.loads(report_path.read_text(encoding="utf-8"))
        fetch = (folder / "fetch.txt").read_text(encoding="utf-8").strip().splitlines() if (folder / "fetch.txt").exists() else []
        out += [""] + repo_section(slug, report, read_rows(data), fetch[-1] if fetch else "not run")
    (root / "docs" / "src" / "benchmarks.md").write_text("\n".join(out) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
