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
        uses: mahkassem/fairlead@v0.6.0
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

*Unreleased:* a program is found the way a shell finds it. On Windows, a name with no extension is looked up on `PATH` with each of `PATHEXT`'s `.com`, `.exe`, `.bat` and `.cmd`, and a relative path such as `node_modules/.bin/eslint` beside the working directory, so `npx` and other `.cmd` shims start. Before, they needed the full name, such as `npx.cmd`, which still works. The same goes for `fairlead done`, `[[guard.external]]` rules and graph providers.

An invocation a [`[[quarantine]]` entry](plan.md#tests-that-lie-on-one-platform) holds doesn't fail the run when its output matches the entry's signature and names no other failed test. It is reported as not provable here, and the last line counts it. Any other failure counts. The entry is checked again on the machine running the plan, so a plan made on Windows excuses nothing on Linux.

`--results PATH` also writes each invocation's id, working directory, argv, outcome and seconds, for `fairlead ci report`, with `"quarantined": true` on one that failed as its entry expects.

### Escapes: `--judge PATH`

Once pull requests run only the plan, the default branch's push run, which still runs everything, is where a missed test shows. `--judge` takes the plan the merged change got, made again against the push's first parent, and judges each failing test file against it:

- **planned**: the merge's plan selected it, so it should have failed on the pull request too;
- **escaped**: the plan left it out. The line names the `[[tests.owners]]` rule that would have caught it, the same rule [`replay report`](replay.md) suggests for a miss, and in GitHub Actions it's also a warning annotation on the test file.

A failing test file is read from the runner's output through that runner's `[[replay.failures]]` extractor and matched to a file in the tree, as `replay` does. A runner with no such entry is named and not judged. Output is still echoed as the runner prints it. With `--results`, the verdicts go in the results file and `ci report` lists the escapes.

```yaml
on: push   # the default branch
steps:
  - uses: actions/checkout@v7
    with:
      fetch-depth: 2
      persist-credentials: false
  - run: |
      bunx fairlead ci plan --base HEAD^ --out "$RUNNER_TEMP/merge.json"
      bunx fairlead ci plan --base HEAD^ --set 'plan.run_all=["**"]' --out "$RUNNER_TEMP/all.json"
  - run: bunx fairlead ci run --plan "$RUNNER_TEMP/all.json" --judge "$RUNNER_TEMP/merge.json" --results "$RUNNER_TEMP/results.json"
  - if: always()
    run: bunx fairlead ci report --plan "$RUNNER_TEMP/all.json" --results "$RUNNER_TEMP/results.json"
```

The run's exit code is its tests' either way. `ci.escapes = "fail"` also makes `ci report` exit 1 when there's an escape, so the escape can be its own required check; the default, `"report"`, only reports it.

## `fairlead ci report --plan PATH`

Writes one Markdown summary of the run. It goes to `$GITHUB_STEP_SUMMARY` when GitHub Actions sets it, else stdout. The summary has:
- the headline: passed, or how many invocations failed, and the time;
- the plan: how many changed files and what it selected, with a count of each reason a test was picked;
- changed files no test reaches, and what the plan did about each;
- the tests and checks not provable where the plan was made, each with its evidence and where it is proved instead;
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
comment = false    # also keep a pull request comment up to date
escapes = "report" # or "fail": `ci report` exits 1 on an escape `ci run --judge` found
```

## GitLab CI

`ci plan`, `ci run` and `ci report` read nothing GitHub sets, so a merge request pipeline runs them as they are. GitLab gives the merge base as `CI_MERGE_REQUEST_DIFF_BASE_SHA` and clones 20 commits by default, which can leave the merge base out: `GIT_DEPTH: "0"` fetches the history `--base` needs.

```yaml
fairlead:
  image: node:22
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
  variables:
    GIT_DEPTH: "0"
    PLAN: /tmp/fairlead-plan.json
    RESULTS: /tmp/fairlead-results.json
  before_script:
    # Pin a release in your own pipeline: releases/download/vX.Y.Z/ in place of releases/latest/download/.
    - curl -fsSL https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.sh | sh
    - export PATH="$HOME/.cargo/bin:$PATH"
    - npm ci
  script:
    - fairlead ci plan --base "$CI_MERGE_REQUEST_DIFF_BASE_SHA" --head "$CI_COMMIT_SHA" --out "$PLAN"
    - fairlead ci run --plan "$PLAN" --results "$RESULTS"
  after_script:
    - export PATH="$HOME/.cargo/bin:$PATH"
    - fairlead ci report --plan "$PLAN" --results "$RESULTS" > fairlead-report.md
  artifacts:
    when: always
    expose_as: fairlead report
    paths: [fairlead-report.md]
```

The plan and results stay outside the checkout, where they'd count as changes; the report is written there last, because GitLab keeps artifacts only from inside it. Without `$GITHUB_STEP_SUMMARY` the report goes to stdout, so it's saved to a file and shown on the merge request through `expose_as`. `--comment` and the `::warning` annotations of `ci run --judge` are GitHub's; on GitLab the escapes are in the report.

A project that installs Fairlead from npm can drop the installer and run `npx fairlead` instead, as on GitHub.
