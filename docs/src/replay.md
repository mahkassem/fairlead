# Replay

Replay is how Fairlead proves a plan's recall: it takes real CI failures from a repository's history, plans each failing commit again, and counts every failing test file or check the plan would have left out. It runs in two phases, so the report can always be reproduced from committed data.

```sh
fairlead replay fetch --repo owner/name --since 2026-07-01 --workflow ci.yml --data bench/data/owner_name.jsonl --clone ../name
fairlead replay run --data bench/data/owner_name.jsonl --clone ../name --config bench/owner_name.toml --fetch-missing
```

## `replay fetch`

Lists every completed `pull_request` and `merge_group` run since `--since`, a week at a time since one listing returns at most 1,000 runs, and records one row per run attempt, passing or failing, so a job that failed and then passed on a re-run shows up as flaky. `--workflow FILE` (repeatable; a workflow's file name, such as `ci.yml`, or its id) lists only those workflows' runs, so other workflows cost no requests, and a first attempt that was cancelled or skipped is left out: it ran nothing, and each recorded attempt costs an API request. It needs `curl` and a token in `GITHUB_TOKEN` or `GH_TOKEN`; in GitHub Actions the workflow's own token can read other public repositories' runs and logs.

- **Failed jobs keep the extractor's input,** not its output: failure-level annotations (GitHub's `.github` exit-code note left out) and the log lines around each `FAIL` or `●`. Logs expire after 90 days; the dataset keeps enough to re-extract with a better extractor later.
- **Each row records its pull request and base.** For a pull request run, the pull request comes from the run, the commit's associated pulls, or, for a fork, the pulls whose head is the fork's branch; the base is the base branch's first-parent commit when the run started, or the default branch's when no pull request is found. For a merge queue run, both come from the queue branch's name (`gh-readonly-queue/<base>/pr-<N>-<sha>`).
- **With `--clone`,** each run's head commit is fetched into the clone, so it's there when `replay run` needs it.
- **Rows already in the dataset are skipped** before any of their jobs are fetched, so a weekly fetch is incremental. `--limit N` stops after N new attempts.
- **GitHub's secondary rate limit,** which refuses bursts with a 403 or 429 however much of the hourly budget is left, is waited out, for API calls and log downloads alike: up to three retries, after the `Retry-After` GitHub sends or one, two and three minutes. Each wait prints a line. An exhausted hourly budget (`X-RateLimit-Remaining: 0`) or any other refusal stops the fetch with GitHub's message.
- **The last line says how it ended:** `fetch complete`, `fetch partial` (the limit was reached; run it again to continue), or `fetch stopped` with the API's error and exit code 2. The rows gathered so far are written in every case, and a report from a partial dataset shouldn't be read as the whole window.
- **The token never reaches curl's arguments,** only its standard input, and a redirect to log storage is followed without it.

The dataset is JSON lines, appended and never rewritten.

## `replay run`

With `--fetch-missing`, the recorded heads and bases the clone lacks are fetched first. `--json-out PATH` writes the JSON report as well as printing the text one. For each failed row in the window, replay checks out the head commit into a worktree beside the clone (created once and reused, so the [parse cache](graph.md) carries over), plans the change from the merge base of the recorded base (or, for a row without one, the clone's default branch as it stood when the run started) and the head, and classes every failure the row names:

| Outcome | Meaning |
| --- | --- |
| hit | the test file or check is in the plan, or the plan selects everything |
| miss | it isn't, and nothing below explains it; recall is hits over hits and misses |
| flaky | another attempt of the same run passed the same job; decided before hit, so a flaky failure never raises recall |
| unconfirmed | a later run of the same change passed the job: the same head again, or the same patch rebased onto a newer base (equal `git patch-id`) |
| unattributed | the job failed, but no test file or check could be named from its annotations or log |
| unavailable | the head commit, or the history to its merge base, isn't in the clone |
| error | the planner refused the commit, such as a test file no runner matches |
| unwatched | no `[[replay.failures]]` or `[[replay.checks]]` entry names the job; listed by job name so a gap in the config can't raise recall |
| quarantined | a `[[replay.quarantine]]` entry declares the test flaky in this job (below) |
| ignored | `replay.ignore` names the job, such as one that only aggregates others, or it failed only in steps `replay.ignore_steps` names, such as an install |

Recall is hits over hits and misses. Strict recall also counts unconfirmed failures as misses.

Failing test files come from annotations and from the log, through the extractors named in `[[replay.failures]]`: `vitest`, `jest`, or `regex` with a pattern that has a named `file` group (and optionally `project` and `title`). A printed path is matched to a file at that commit: as it stands, under the named project or package, by a unique suffix, or among the suffix matches by the one whose source contains the failing test's title. Anything still ambiguous is unattributed. Failing checks come from `[[replay.checks]]`, which map a job and step name to a check id.

The window is the `replay.window_days` days ending at the newest recorded run, or at `--until`, so the same dataset always gives the same report. The report says whether `replay.min_failures` attributed failures were reached.

```toml
[replay]
window_days = 90
min_failures = 30
ignore = ["^all-green$"]

[[replay.failures]]
runner = "vitest"
extractor = "vitest"
job = "^test"

[[replay.checks]]
job = "^lint$"
step = "^Typecheck$"
check = "typecheck"
```

## Quarantine

A test that fails in one CI job whatever changes, such as a platform-specific flake, measures that job rather than the planner. A `[[replay.quarantine]]` entry declares it:

```toml
[[replay.quarantine]]
path = "test/typecheck.test.ts"     # one test file
job = "^unit, windows$"             # the jobs it fails in, as a regex
reason = "Fails only on Windows: 60 failures across 36 pull requests; the job passed 356 times."
until = "2026-12-31"                # it stops applying after this day
```

Every failure is still planned and judged. While an entry applies, its matching hits, misses and unconfirmed failures become `quarantined`, and the report says what each would have been. It applies only while the dataset bears it out: the test failed in at least three distinct pull requests in the window, and never in a job the entry doesn't name. Otherwise it's reported `unverified`. After `until` it's `expired`, and when it matches nothing it's `stale`. The report gives recall both ways, with the number of failures each is measured on: raw (quarantined failures counted as what they would have been) and adjusted (left out). Quarantined failures don't count toward `replay.min_failures`.

## The report

Per repository: runs replayed, attributed failures against the gate, recall and strict recall (overall and per event, since merge queue runs usually run everything and are the better evidence), hits by how they were selected (by the plan, because the plan selected everything, or as a check), the other outcomes, unwatched jobs by name, the median and 90th-percentile share of test files selected, the share of plans that selected everything, and the first (cold), median and 90th-percentile planning time. Each miss lists the changed files and an owner rule that would have caught it. `--json` prints the same as JSON.

## Limits

- CI ran a pull request's merge commit with its base, while replay plans the head against the merge base. A failure caused by the base alone can show up as a miss.
- A repository whose pull request CI already runs only affected tests records only what it chose to run; its merge queue runs, which usually run everything, are the better evidence, and "passed at a later push" is weaker there.

## The benchmarks

`bench/` holds a config for each public benchmark repository, written from how that repository's CI runs its tests. The `bench` workflow runs weekly, or by hand with `since` (85 days back by default, since GitHub keeps job logs for 90) and `limit`:

- For each repository in turn, it builds Fairlead from the workflow's commit, starts from the dataset on the `bench-data` branch (or `main`), clones the repository with full history and no blobs, records new runs, and replays the window with `--fetch-missing`.
- A last job copies each grown dataset into `bench/data/`, writes [Benchmarks](benchmarks.md), and force-pushes both to `bench-data`, recreated from `main` each time, then opens a pull request from it if none is open.
- The workflow token's API budget is about 1,000 requests an hour for the whole run, so a first backfill takes several runs: each records up to `limit` attempts per repository and the next carries on. A `BENCH_READ_TOKEN` secret, if set, is used instead. Pull requests opened by the workflow token don't start CI.
