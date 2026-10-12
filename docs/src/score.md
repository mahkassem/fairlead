# Readiness score

*Since 0.10.0*, `fairlead doctor --score` says how ready a repository is for Fairlead, as points out of 100 on a fixed rubric. Each item asks for evidence that the part works, not only that it's set up: a done gate that never passed, or hooks that never saw a write, earn nothing. The rubric has a version, and a score is only compared with one on the same version, so a team can record a score before onboarding and again after.

```sh
fairlead doctor --score               # the items, then the three fixes worth the most
fairlead doctor --score --json        # the same, as JSON
fairlead doctor --score --min 70      # exits 1 below 70, for CI
fairlead doctor --score --replay out/report.json
```

## Rubric 1

| Item | Points | Earned when |
|---|---|---|
| `config` | 10 | a project file loads with no problems |
| `runners` | 15 | there's a runner, and every test file has exactly one |
| `hooks` | 10 | Claude Code's hooks are installed, and Codex's or Gemini CLI's where `.codex/` or `.gemini/` exists; split between them |
| `done` | 10 | `fairlead done` passed in the last 30 days |
| `ci` | 10 | 5 for a workflow that runs `fairlead ci` (or the action), 5 more with `[stages]` |
| `replay` | 15 | a replay report whose window ends in the last 30 days and met its `min_failures` gate |
| `recall` | 15 | that report's recall is 95% or more, and its median plan selected less than everything |
| `firing` | 5 | the write hook checked a write in the last 7 days |
| `skills` | 5 | there's a routed skill, and every SKILL.md has a route |
| `lessons` | 5 | there's a lesson, and none is due for review |

The replay items read a `fairlead replay run --json-out` report: `--replay FILE`, or `.fairlead/replay.json` when it's there. A repository whose replay runs in CI can download that report before scoring. Recall counts only beside a median plan that selects less than everything, so `plan.run_all = ["**"]` can't earn it.

The events behind `done`, `firing` and `hooks` are the local event log, so a score reflects the machine it runs on. The tracker and token items in the design come with them, in rubric 2.
