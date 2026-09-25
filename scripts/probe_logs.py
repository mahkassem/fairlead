#!/usr/bin/env python3
"""Probes whether a token can read another public repository's Actions data:
recent failed pull request runs, their jobs, check-run annotations and raw job
logs. Writes a summary table and keeps short excerpts of failure lines, which
become extractor fixtures.

    GH_TOKEN=... REPOS="owner/a owner/b" COUNT=10 OUT=out python3 scripts/probe_logs.py
"""
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

API = "https://api.github.com"
REPO = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
TIMESTAMP = re.compile(r"^\d{4}-\d{2}-\d{2}T[\d:.]+Z ")
FAIL = re.compile(r"\bFAIL\b")
LINES_AFTER_FAIL = 2
MAX_EXCERPT_LINES = 40
MAX_COUNT = 50


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def request(url: str, token: str | None, accept: str = "application/vnd.github+json"):
    """Status, headers and body. Redirects aren't followed, so a token never
    goes to the storage host a log download redirects to."""
    headers = {"Accept": accept, "User-Agent": "fairlead-log-probe"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    opener = urllib.request.build_opener(NoRedirect)
    try:
        with opener.open(urllib.request.Request(url, headers=headers), timeout=60) as resp:
            return resp.status, resp.headers, resp.read()
    except urllib.error.HTTPError as err:
        return err.code, err.headers, err.read()


def api_json(path: str, token: str | None):
    status, _, body = request(f"{API}{path}", token)
    return status, (json.loads(body) if status == 200 else None)


def job_log(repo: str, job_id: int, token: str | None) -> tuple[int, str]:
    status, headers, body = request(f"{API}/repos/{repo}/actions/jobs/{job_id}/logs", token)
    if status in (301, 302, 307) and headers.get("Location"):
        status, _, body = request(headers["Location"], None, accept="*/*")
    return status, body.decode("utf-8", "replace") if status == 200 else ""


def excerpt(log: str) -> list[str]:
    lines = [TIMESTAMP.sub("", ANSI.sub("", line)) for line in log.splitlines()]
    keep: list[str] = []
    for i, line in enumerate(lines):
        if FAIL.search(line):
            keep.extend(lines[i : i + 1 + LINES_AFTER_FAIL])
        if len(keep) >= MAX_EXCERPT_LINES:
            break
    return keep[:MAX_EXCERPT_LINES]


def probe(repo: str, count: int, token: str | None, out: Path) -> list[str]:
    rows = []
    status, runs = api_json(f"/repos/{repo}/actions/runs?status=failure&event=pull_request&per_page={count}", token)
    if runs is None:
        return [f"| {repo} | runs {status} | | | |"]
    for run in runs.get("workflow_runs", []):
        status, jobs = api_json(f"/repos/{repo}/actions/runs/{run['id']}/jobs?filter=latest", token)
        failed = [j for j in (jobs or {}).get("jobs", []) if j.get("conclusion") == "failure"]
        for job in failed[:3]:
            a_status, notes = api_json(f"/repos/{repo}/check-runs/{job['id']}/annotations", token)
            l_status, log = job_log(repo, job["id"], token)
            lines = excerpt(log)
            if lines or notes:
                target = out / repo.replace("/", "__") / f"{job['id']}.txt"
                target.parent.mkdir(parents=True, exist_ok=True)
                notes_text = [f"{n.get('path')}:{n.get('start_line')}: {n.get('title') or ''}" for n in (notes or [])]
                target.write_text("\n".join([f"# run {run['id']} job {job['id']} ({job['name']})", *notes_text, "---", *lines]) + "\n")
            rows.append(f"| {repo} | {run['id']} | {job['name']} | annotations {a_status} ({len(notes or [])}) | log {l_status} ({len(lines)} FAIL lines) |")
    return rows or [f"| {repo} | no failed PR runs listed | | | |"]


def main() -> int:
    token = os.environ.get("GH_TOKEN") or None
    repos = os.environ.get("REPOS", "").split()
    count = int(os.environ.get("COUNT", "10"))
    out = Path(os.environ.get("OUT", "out"))
    if not 1 <= count <= MAX_COUNT or not repos or not all(REPO.match(r) for r in repos):
        print("REPOS must be owner/name entries and COUNT 1 to 50", file=sys.stderr)
        return 2
    table = ["| repo | run | job | annotations | log |", "| --- | --- | --- | --- | --- |"]
    for repo in repos:
        table.extend(probe(repo, count, token, out))
    report = "\n".join(table)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(f"### {out.name}\n\n{report}\n\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
