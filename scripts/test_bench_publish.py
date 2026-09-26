import json
import pathlib
import tempfile
import unittest

import bench_publish


def row(run_id, attempt=1, day="2026-09-10"):
    return {"run_id": run_id, "attempt": attempt, "created_at": f"{day}T10:00:00Z"}


def report(**over):
    base = {
        "repo": "o/r", "from": "2026-06-29", "until": "2026-09-26", "runs": 3,
        "hits": 2, "hits_selected": 1, "hits_run_all": 1, "hits_check": 0,
        "misses": [{"run_id": 7, "attempt": 1, "target": "t`x|y.test.ts",
                    "changed": ["src/a.ts"], "fix": ""}],
        "flaky": 0, "unconfirmed": 1, "unattributed": 0, "unavailable": 0,
        "errors": 0, "ignored": 0, "unwatched": {"docs | `x`": 2},
        "widened_by": {"run-all pnpm-lock.yaml": 3},
        "judged": 3, "raw_recall": 0.5, "raw_judged": 6, "quarantined": 3,
        "quarantine": [{"path": "test/a.test.ts", "job": "^win$", "reason": "fails | on `win`",
                        "until": "2026-12-31", "status": "active", "pulls": 3, "other_jobs": [],
                        "absorbed": 3, "would_hit": 0, "would_miss": 3, "would_unconfirmed": 0}],
        "recall": 2 / 3, "strict_recall": 0.5, "min_failures": 30,
        "by_event": {"pull_request": {"recall": 2 / 3, "strict_recall": 0.5}},
        "median_selected": 0.1, "p90_selected": 0.4, "run_all_share": 0.25,
        "median_plan_seconds": 0.2, "p90_plan_seconds": 0.5, "first_plan_seconds": 1.1,
    }
    base.update(over)
    return base


class BenchPublishTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "bench" / "data").mkdir(parents=True)
        (self.root / "docs" / "src").mkdir(parents=True)
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "9.9.9"\n')
        (self.root / "bench" / "o_r.toml").write_text("")
        self.art = self.root / "artifacts" / "bench-o_r"
        self.art.mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def write_rows(self, path, rows):
        path.write_text("".join(json.dumps(r) + "\n" for r in rows))

    def run_publish(self):
        bench_publish.main([str(self.root / "artifacts"), "--root", str(self.root), "--commit", "abcdef1234567890"])
        return (self.root / "docs" / "src" / "benchmarks.md").read_text()

    def test_a_grown_dataset_replaces_the_old_one_and_the_page_is_written(self):
        data = self.root / "bench" / "data" / "o_r.jsonl"
        self.write_rows(data, [row(1)])
        self.write_rows(self.art / "data.jsonl", [row(1), row(2, day="2026-09-20")])
        (self.art / "report.json").write_text(json.dumps(report()))
        (self.art / "fetch.txt").write_text("fetch complete: 1 new rows in out/data.jsonl\n")
        page = self.run_publish()
        self.assertEqual(len(data.read_text().splitlines()), 2)
        self.assertIn("Fairlead 9.9.9 at `abcdef123456`", page)
        self.assertIn("2 run attempts from 2026-09-10 to 2026-09-20", page)
        self.assertIn("| Recall | 66.7% |", page)
        self.assertIn("| Strict recall (unconfirmed as misses) | 50.0% |", page)
        self.assertIn("fetch complete", page)
        self.assertIn("- `run-all pnpm-lock.yaml`: 3", page)
        self.assertIn("(raw) | 50.0% (n=6) |", page)
        self.assertIn("(adjusted) | 66.7% (n=3) |", page)
        self.assertIn("`test/a.test.ts` in jobs `^win$`: active, 3 failures absorbed (0 would-be hits, 3 would-be misses)", page)

    def test_the_headline_numbers_are_written_for_the_site(self):
        (self.art / "report.json").write_text(json.dumps(report()))
        self.run_publish()
        summary = json.loads((self.root / "docs" / "src" / "benchmarks.json").read_text())
        self.assertEqual(summary["fairlead"], "9.9.9")
        self.assertEqual(summary["commit"], "abcdef123456")
        [repo] = summary["repos"]
        self.assertEqual((repo["repo"], repo["judged"], repo["raw_judged"]), ("o/r", 3, 6))
        self.assertAlmostEqual(repo["recall"], 2 / 3)
        self.assertFalse(repo["gate_met"])

    def test_a_dataset_that_drops_rows_is_refused(self):
        data = self.root / "bench" / "data" / "o_r.jsonl"
        self.write_rows(data, [row(1), row(2)])
        self.write_rows(self.art / "data.jsonl", [row(2), row(3)])
        self.run_publish()
        self.assertIn('"run_id": 1', data.read_text())

    def test_an_empty_dataset_is_not_committed(self):
        (self.art / "data.jsonl").write_text("")
        self.run_publish()
        self.assertFalse((self.root / "bench" / "data" / "o_r.jsonl").exists())

    def test_text_from_other_repositories_cannot_break_out_of_code_spans(self):
        (self.art / "report.json").write_text(json.dumps(report()))
        page = self.run_publish()
        self.assertIn("`t x y.test.ts`", page)
        self.assertIn("`docs    x `: 2", page)
        for line in page.splitlines():
            if line.startswith("- "):
                self.assertEqual(line.count("`") % 2, 0, line)


if __name__ == "__main__":
    unittest.main()
