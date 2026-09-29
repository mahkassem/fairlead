# Plans in CI

`fairlead ci plan` makes the [test plan](plan.md) for the commit a CI job checked out, writes it as JSON, and with `--format github` sets step outputs. `fairlead ci run --plan` runs the plan's invocations. The plan reads the working tree, so check out the commit you want planned, with enough history for the merge base:

```yaml
on: pull_request
permissions:
  contents: read
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          ref: ${{ github.event.pull_request.head.sha }}   # the head, not the merge commit
          fetch-depth: 0                                    # history for the merge base
          persist-credentials: false
      - id: plan
        uses: mahkassem/fairlead@v0.5.0
        with:
          command: >-
            ci plan --format github
            --base ${{ github.event.pull_request.base.sha }}
            --head ${{ github.event.pull_request.head.sha }}
      - run: npm ci
        if: steps.plan.outputs.tests != '0' || steps.plan.outputs.checks != ''
      - run: fairlead ci run --plan "$PLAN"
        env:
          PLAN: ${{ steps.plan.outputs.plan }}
```

Pin the action and `actions/checkout` to a commit SHA in your own workflows.

## `fairlead ci plan`

| Flag | Default | |
| --- | --- | --- |
| `--base SHA` | the remote's default branch | The merge base of this and `HEAD` is what changes are measured from. |
| `--head SHA` | none | Refuses to plan if `HEAD` is another commit. `actions/checkout` checks out the merge commit on `pull_request` unless you pass `ref`. |
| `--format json\|github` | `json` | `github` also writes step outputs. |
| `--out PATH` | `fairlead-plan.json` in `$RUNNER_TEMP` or the system temp directory | Never the working tree, where the file would count as a change. A relative path is from the current directory. |
| `--set KEY=VALUE` | | Override a config value for this run. |

A missing merge base fails with exit code 2, never an empty plan: fetch more history or pass `--files`.

### Step outputs

| Output | Value |
| --- | --- |
| `all` | `true` when everything is selected |
| `plan` | the path to the plan JSON |
| `plan_id` | the plan's id |
| `invocations` | the invocations as a JSON array; `fromJSON` makes it a job matrix |
| `checks` | the selected check ids, space-separated |
| `tests` | how many test files are selected |

Nothing is keyed to a runner name or folder layout. `invocations` can feed a job matrix with `fromJSON`; pass each argv to a program through an environment variable rather than writing it into a `run:` line, which would let a path in the plan be read as shell syntax. For most projects, `fairlead ci run --plan` in one job is simpler.

## `fairlead ci run --plan PATH`

Runs each invocation's argv in its working directory (relative to the repository root; a relative `--plan` is from the current directory), in order, without a shell, and exits 1 if any failed or has no argv, after running the rest. `--fail-fast` stops at the first failure. A plan of another version is refused with exit code 2.

Without a shell, Windows finds only `.exe` programs on `PATH`: a runner that starts `npx` or another `.cmd` shim needs the full name, such as `npx.cmd`, in its argv.

`--results PATH` also writes each invocation's id, working directory, argv, outcome and seconds, for `fairlead ci report`.

## `fairlead ci report --plan PATH`

Writes one Markdown summary of the run. It goes to `$GITHUB_STEP_SUMMARY` when GitHub Actions sets it, else stdout. The summary has:
- the headline: passed, or how many invocations failed, and the time;
- the plan: how many changed files and what it selected, with a count of each reason a test was picked;
- changed files no test reaches, and what the plan did about each;
- with `--results`, a line per invocation with its outcome and time, and a block with the command to run each failed one again;
- with `--receipt FILE`, the [receipt](receipt.md) a branch carries, as `fairlead receipt --out` wrote it, folded away.

Results from another plan are refused with exit code 2.

```yaml
      - run: bunx fairlead ci run --plan "$PLAN" --results "$RUNNER_TEMP/results.json"
      - if: always()
        run: bunx fairlead ci report --plan "$PLAN" --results "$RUNNER_TEMP/results.json"
```

`--comment`, or `ci.comment = true`, also keeps one pull request comment up to date with the same report. The comment is found again by a hidden marker at its start, so each run edits it rather than adding another. It needs `GITHUB_TOKEN` with `pull-requests: write`, and it's read from a `pull_request` event's payload. A comment that can't be posted is reported and never fails the step, since the summary already has the report.

```toml
[ci]
comment = false   # also keep a pull request comment up to date
```
