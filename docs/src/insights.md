# Insights

*Since 0.10.0*, `fairlead insights` sums up how the setup has been working, and `--suggest` lists the config changes that evidence backs.

```sh
fairlead insights                                   # the last 30 days of the event log
fairlead insights --since 2026-09-01 --json
fairlead insights --replay out/report.json --results results.json --results results-2.json
fairlead insights --suggest                         # and the changes the evidence backs
fairlead insights --suggest --write                 # and apply the one-key change
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
| a `[[replay.quarantine]]` entry | a recurring test in the replay report | no: a person confirms it's flaky and gives the reason |
| drop or narrow a skill route | offered in ten measured sessions and never used | no |
| `guard.on_finding = "deny"` | twenty warned writes, and no finding reached a commit | yes |

`--write` applies only the guard change, because it's the one that changes a single key. It edits that line in `fairlead.toml`, or adds it under `[guard]`, so the file's comments and order stay as they were. The rest are printed for a person to judge. Nothing is committed.

## Weekly summary

`fairlead ci workflow --insights --write` writes `.github/workflows/fairlead-insights.yml`. That workflow runs on Mondays at 05:17 UTC, or by hand, and does three things:

1. Replays the last 30 days of the `fairlead.yml` workflow's runs.
2. Runs `fairlead insights --since` seven days ago, with that replay report and `--suggest`.
3. Puts the result in the run's summary.

It runs no tests. It's kept apart from `fairlead.yml` so the schedule never starts a full test run, and it needs only `contents: read` and `actions: read`. It sits on CI's side, so the event log's sections are empty there: the replay's recall, misses and recurring tests carry the summary. Run `fairlead insights` locally for the event log. As with `fairlead.yml`, `--write` replaces only a file this command wrote.
