# Replay

Replay is how Fairlead proves a plan's recall: it takes real CI failures from a repository's history, plans each failing commit again, and counts every failing test file or check the plan would have left out. It runs in two phases, so the report can always be reproduced from committed data.

```sh
fairlead replay fetch --repo owner/name --since 2026-07-01 --data bench/data/owner_name.jsonl --clone ../name
fairlead replay run --data bench/data/owner_name.jsonl --clone ../name --config bench/owner_name.toml
```

## `replay fetch`

Lists every completed `pull_request` and `merge_group` run since `--since`, and records one row per run attempt, passing or failing, so a job that failed and then passed on a re-run shows up as flaky. It needs `curl` and a token in `GITHUB_TOKEN` or `GH_TOKEN`; in GitHub Actions the workflow's own token can read other public repositories' runs and logs.

- **Failed jobs keep the extractor's input,** not its output: failure-level annotations (GitHub's `.github` exit-code note left out) and the log lines around each `FAIL` or `●`. Logs expire after 90 days; the dataset keeps enough to re-extract with a better extractor later.
- **Each row records its pull request and base.** For a pull request run, the pull request comes from the commit's associated pulls and the base is the base branch's first-parent commit when the run started. For a merge queue run, both come from the queue branch's name (`gh-readonly-queue/<base>/pr-<N>-<sha>`).
- **With `--clone`,** each run's head commit is fetched into the clone, so it's there when `replay run` needs it.
- **Rows already in the dataset are skipped** before any of their jobs are fetched, so a weekly fetch is incremental. On a rate limit or other API error, the rows gathered so far are still written.
- **The token never reaches curl's arguments,** only its standard input, and a redirect to log storage is followed without it.

The dataset is JSON lines, appended and never rewritten.

## `replay run`

For each failed row in the window, replay checks out the head commit into a worktree beside the clone (created once and reused, so the [parse cache](graph.md) carries over), plans the change from the merge base of the recorded base and the head, and classes every failure the row names:

| Outcome | Meaning |
| --- | --- |
| hit | the test file or check is in the plan, or the plan selects everything |
| miss | it isn't, and nothing below explains it; recall is hits over hits and misses |
| flaky | another attempt of the same run passed the same job |
| unconfirmed | a later push of the same pull request passed the job, and the change between the two heads doesn't reach it |
| unattributed | the job failed, but no test file or check could be named from its annotations or log |
| unavailable | the head commit, or the history to its merge base, isn't in the clone |
| error | the planner refused the commit, such as a test file no runner matches |

Failing test files come from annotations and from the log, through the extractors named in `[[replay.failures]]`: `vitest`, `jest`, or `regex` with a pattern that has a named `file` group (and optionally `project` and `title`). A printed path is matched to a file at that commit: as it stands, under the named project or package, by a unique suffix, or among the suffix matches by the one whose source contains the failing test's title. Anything still ambiguous is unattributed. Failing checks come from `[[replay.checks]]`, which map a job and step name to a check id.

The window is `replay.window_days` ending at the newest recorded run, or at `--until`, so the same dataset always gives the same report. The report says whether `replay.min_failures` attributed failures were reached.

```toml
[replay]
window_days = 90
min_failures = 30

[[replay.failures]]
runner = "vitest"
extractor = "vitest"
job = "^test"

[[replay.checks]]
job = "^lint$"
step = "^Typecheck$"
check = "typecheck"
```

## The report

Per repository: runs replayed, attributed failures against the gate, recall, the other outcomes, the median and 90th-percentile share of test files selected, the share of plans that selected everything, and the median planning time. Each miss lists the changed files and an owner rule that would have caught it. `--json` prints the same as JSON.

## Limits

- CI ran a pull request's merge commit with its base, while replay plans the head against the merge base. A failure caused by the base alone can show up as a miss.
- A repository whose pull request CI already runs only affected tests records only what it chose to run; its merge queue runs, which usually run everything, are the better evidence, and "passed at a later push" is weaker there.
