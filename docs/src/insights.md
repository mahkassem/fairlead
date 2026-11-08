# Insights

*Since 0.10.0*, `fairlead insights` sums up how the setup has been working, and `--suggest` lists the config changes that evidence backs.

```sh
fairlead insights                                   # the last 30 days of the event log
fairlead insights --since 2026-09-01 --json
fairlead insights --replay out/report.json --results results.json --results results-2.json
fairlead insights --suggest                         # and the changes the evidence backs
fairlead insights --suggest --write                 # and apply the one-key change
fairlead insights --speed --dataset runs.jsonl      # where the time goes (Unreleased)
```

## What it reads

| Section | From |
|---|---|
| `guard` | the write hook's decisions and denies by rule, the same counts `fairlead doctor` shows |
| `done` | `fairlead done` runs passed and failed, and the steps that failed |
| `skills` | skill routing's hit rate, with the skills offered and unused, and those used without an offer (as `fairlead skills report` counts them) |
| `lessons` | lessons no brief has offered in the window |
| `tokens` | sessions and their cost, from the newest record of each ([Tokens and cost](tokens.md)) |
| `replay` | with `--replay`, a `replay run --json-out` report's recall, misses and recurring tests |
| `escapes` | with `--results`, the escapes in `ci run --results` files, one per merge |

The event log is local, so the first five sections reflect the machine `insights` runs on. A replay report and results files usually come from CI.

## Suggestions

`--suggest` lists only changes the evidence backs. Each comes with its counts:

| Suggestion | Evidence | `--write` |
|---|---|---|
| an `[[tests.owners]]` rule | two or more escapes or replay misses needed the same rule | no: add it, then replay to see what it costs in plan size |
| a `[[replay.quarantine]]` entry | a recurring test in the replay report; *Unreleased:* or a test a [`ci ledger`](ci.md#is-this-red-mine-fairlead-ci-ledger) found flaky, from the cached ledgers made since `--since` and any `--ledger FILE`, each named once with how many ledgers found it | no: a person confirms it's flaky and gives the reason |
| drop or narrow a skill route | offered in ten measured sessions and never used | no |
| `guard.on_finding = "deny"` | twenty warned writes, and no finding reached a commit | yes |
| a runner's `command` in another Bun test mode | *Since 0.14.0:* the last [`fairlead test bench`](plan.md#measuring-buns-test-modes) recommended that mode for the runner, with each mode's mean time in the suggestion | no: the times are one machine's, so a person decides |

`--write` applies only the guard change, because it's the one that changes a single key. It edits that line in `fairlead.toml`, or adds it under `[guard]`, so the file's comments and order stay as they were. The rest are printed for a person to judge. Nothing is committed.

## Speed

*Unreleased:* `fairlead insights --speed` says where the time goes, from what Fairlead already records: `fairlead done`'s per-step seconds and the time it took to plan, from the event log since `--since`; the invocations of each `--results` file; and the CI jobs, reruns and cancelled runs of each `--dataset FILE`, a [`replay fetch`](replay.md#replay-fetch) dataset. Without a plan time from `done`, a `--replay` report's median planning time stands in. Each line carries its numbers and one change to try; `--json` prints them under `speed`. Nothing is written.

| Line | When | Suggestion |
|---|---|---|
| `slowest` | the three steps or jobs with the most time in all, with their median and 90th percentile | split a CI step with `ci plan --shards 2`; keep a local one out of changes that don't need it |
| `never-failed` | a runner or check that passed 20 runs in a row | `from = "merge"`: a candidate for a later stage |
| `always-fails` | a step that failed 5 or more runs, every one | fix it, or record what fails today: `[[replay.quarantine]]`, `[[quarantine]]` or `guard.baseline` |
| `run-whole` | a runner whose longest run, over 3 or more, took no longer than planning | `all_from = "draft"`: choosing its tests saves less than planning costs |
| `reruns` | a dataset's reruns, and the runs that passed only on a rerun of the same commit | `fairlead ci ledger` names the flaky test or the platform |
| `cancelled` | a dataset's cancelled runs and their share | keep the pull request a draft while pushing often |
| `pulls` | the three pull requests with the most reruns and cancellations | look at those first |

```text
speed
  plan     median 1.2 s to plan a change
  slowest      test (CI job): median 5.0 min, p90 5.0 min over 2 runs, 9.7 min in all
    → split it across jobs with `fairlead ci plan --shards 2`, balanced by `--durations`
  run-whole    unit (done): its longest run took 0.3 s against 1.2 s to plan
    → choosing its tests saves less than planning costs: `[[tests.runners]] id = "unit", all_from = "draft"` runs it whole at every stage
```

Datasets fetched before this release have no times and no cancelled runs, so their lines are missing until the next fetch; `done` runs before it recorded no plan time.

## Weekly summary

`fairlead ci workflow --insights --write` writes `.github/workflows/fairlead-insights.yml`. That workflow runs on Mondays at 05:17 UTC, or by hand, and does three things:

1. Replays the last 30 days of the `fairlead.yml` workflow's runs.
2. Runs `fairlead insights --since` seven days ago, with that replay report and `--suggest`.
3. Puts the result in the run's summary.

It runs no tests. It's kept apart from `fairlead.yml` so the schedule never starts a full test run, and it needs only read scopes: `contents` and `actions`, plus `pull-requests` and `checks` for `replay fetch` (a run's pull request and its annotations). It sits on CI's side, so the event log's sections are empty there: the replay's recall, misses and recurring tests carry the summary. Run `fairlead insights` locally for the event log. As with `fairlead.yml`, `--write` replaces only a file this command wrote.
