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
        uses: mahkassem/fairlead@v0.9.0
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
| `--stage auto\|none\|draft\|ready\|merge\|full` | `auto` for a config that uses [stages](#stages), else none | *Since 0.8.0:* the stage to plan for. `none` plans as before stages existed. |
| `--since-green JOB` | none | *Since 0.8.0:* at the merge stage, plan from the last commit where the job JOB passed, so one run covers the batch merged since ([batching](#batching-after-merge---since-green)). Repeatable. Ignored at other stages. |

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
| `stage` | *Since 0.8.0:* the plan's [stage](#stages), empty without one |
| `reused` | *Since 0.8.0:* at merge, the pull request whose passing run of this exact tree the [ready steps were reused](#reuse-at-merge) from, empty otherwise |
| `run_<id>` | *Since 0.8.0:* `true` or `false` for every runner and check: whether this plan runs it. An id's characters other than letters, digits and `_` become `_`, so `desktop-build` is `run_desktop_build`; two ids that become one name fail the plan |

Nothing is keyed to a runner name or folder layout. `invocations` can feed a job matrix with `fromJSON`; pass each argv to a program through an environment variable rather than writing it into a `run:` line, which would let a path in the plan be read as shell syntax. For most projects, `fairlead ci run --plan` in one job is simpler.

## Stages

*Since 0.8.0:* a change goes through stages, and each runner and check says the earliest one it runs at with `from`. It runs at that stage and every later one: `draft` < `ready` < `merge` < `full`.

```toml
[stages]
environments = ["main"]        # pushes here are the merge stage; add "staging", "production"
full_label = "run-everything"  # a pull request label that runs everything

[[tests.runners]]
id = "unit"
match = ["src/**/*.test.ts"]
command = ["bunx", "vitest", "run", "{files}"]
from = "draft"                 # draft | ready (a runner's default) | merge | full

[[tests.runners]]
id = "e2e"
match = ["e2e/**/*.spec.ts"]
command = ["bunx", "playwright", "test", "{files}"]
from = "merge"

[[checks]]
id = "desktop-build"
command = ["./scripts/build-desktop.sh"]
from = "merge"                 # a check's default is draft
```

`ci plan` reads the stage from the GitHub event:

| Event | Stage |
| --- | --- |
| `pull_request` or `pull_request_target`, a draft | `draft` |
| `pull_request` or `pull_request_target`, not a draft (any action, `ready_for_review` included) | `ready` |
| a pull request labelled `stages.full_label` | `full` |
| `push` to a branch in `stages.environments` (a name or a glob such as `release/*`) | `merge` |
| `merge_group` | `merge` |
| `push` to any other branch, a tag, or another event | `ready` |
| `schedule` or `workflow_dispatch` | `full` |

Outside GitHub Actions, `auto` is `ready`. The first line of output says which stage and why, such as `stage: draft (auto: pull_request, a draft)`.

The plan still decides which tests run; the stage decides whether a runner or check runs at all. A later stage's steps leave the plan's invocations, tests and checks and are listed in its `deferred`, each with its `from` and how many tests or checks it would have run, so a step waiting for its stage reads differently from one the change didn't reach. `full` plans everything whatever changed. Each stage of a tree has its own `plan_id`. `[stages]` and `from` are left out of `config_digest`: they say when a step runs, not what it proves.

A config with no `[stages]` and no `from` plans exactly as before, with no stage and the same plan ids. So does `--stage none`.

### Reuse at merge

With `stages.reuse` (on by default once `[stages]` is set), a merge doesn't run again what its pull request already passed on the same bytes:

- At the ready stage, once `ci run` has run every step of the plan and all of them passed, it sets the commit status `fairlead/tree` on the pull request's head commit, saying which tree it tested and the config digest it planned with. That needs `statuses: write` on the job. Without it, or without a token, the run still passes and says the merge will run these steps again.
- At the merge stage, `ci plan` finds the pull request the pushed commit came from and reads that status. When the pushed tree and the config digest are exactly the ones it recorded, the ready stage's steps leave the plan, the plan's `reused` names the pull request and the steps, and the run says `reuse: #7 passed this tree; skipping unit`. The merge stage's own steps, such as end-to-end, still run.
- Anything else runs them: another tree (the base moved before the merge, so the merged bytes were never tested together), another config, a run that skipped some of its steps with `--only` or `--except`, a failed or held step, a working tree that isn't a commit, or an API error. A merge commit, a squash and a rebase all give the same tree when the base hadn't moved.
### Batching after merge: `--since-green`

*Since 0.8.0:* a slow step from the `merge` stage, such as an end-to-end runner, can run once per batch of merged changes instead of once per push. At the merge stage, `fairlead ci plan --since-green e2e` plans from the head commit of the newest earlier run of this workflow on this branch where the job named `e2e` concluded `success`. The plan then covers every change merged since that job last passed, whatever runs were cancelled, skipped or failed in between. JOB is the job's name as the Actions API reports it: its `name:`, or its id when it has none. With the flag repeated, the base is the newest commit at which every named job had passed.

It reads `GITHUB_REPOSITORY`, `GITHUB_RUN_ID`, `GITHUB_API_URL` and a token from `GITHUB_TOKEN` or `GH_TOKEN`, which needs `actions: read`. It finds this run's workflow, lists up to 100 of that workflow's runs on the branch, newest first, and reads each run's jobs until the job has passed. Only earlier `push` and `merge_group` runs on the same branch count: a pull request run tested a commit that never landed as such. The chosen base and its run are printed, so a failing run names its batch, everything between that commit and `HEAD`:

```text
since-green: base 1f2e3d4c5b6a from run 912 (#41), the newest where e2e passed; this batch is 1f2e3d4c5b6a..HEAD (https://github.com/OWNER/REPO/actions/runs/912)
```

Any doubt about the base plans everything, with one line saying why and a plan warning:

| Warning | When |
| --- | --- |
| `since-green-unavailable` | no token, not a GitHub Actions run, or the API failed or was rate-limited |
| `since-green-not-found` | none of the last 100 runs on the branch has the job passing |
| `since-green-not-in-history` | the commit isn't in the clone (check out with `fetch-depth: 0`), or isn't an ancestor of `HEAD` |

At any other stage the flag is ignored with a one-line note, so one plan command serves every event.

### A staged workflow: `fairlead ci workflow`

*Since 0.8.0:* `fairlead ci workflow` prints a GitHub Actions workflow made from the config, and `fairlead ci workflow --write` writes it to `.github/workflows/fairlead.yml`. For the config above it has:

- **Triggers:** `pull_request` (opened, synchronize, reopened, ready_for_review, converted_to_draft, labeled and unlabeled, so the stage follows the pull request), `push` to `stages.environments`, and `workflow_dispatch`.
- **`plan`:** checks out the whole history, installs Fairlead with the action at this release, runs `fairlead ci plan --format github` with `--since-green` for each job of its own, and uploads the plan as the `fairlead-plan` artifact. Its outputs are `stage` and every `run_<id>`.
- **`checks`:** runs the plan with `--except` for each step that has a job of its own. A newer push to the same pull request or branch cancels it.
- **A job per runner or check whose `from` is `merge` or `full`,** here `e2e` and `desktop-build`: it runs only when `needs.plan.outputs.run_<id> == 'true'`, with `fairlead ci run --only <id>`. Its concurrency group is per workflow, job and branch, with `cancel-in-progress: false`, which is GitHub's own batching: one run at a time and only the newest waiting. A waiting run a newer push replaces is covered by the newer run's plan, since both plan from the last green run.
- **Permissions:** `contents: read` at the top and in each job; `plan` adds `actions: read` for `--since-green`. With `stages.reuse`, `plan` also gets `pull-requests: read` and `statuses: read` to find the tree a merge's pull request passed, and `checks` gets `statuses: write` to [record it](#reuse-at-merge). Every action is pinned to a commit, and no `github.event` text is written into a `run:` line: the pull request's base reaches the shell through an environment variable.

The file starts with a comment marking it as generated by `fairlead ci workflow`. `--write` rewrites a file with that comment and refuses any other, naming it, so a workflow you wrote is never replaced. A gated id may hold only letters, digits, `-`, `_` and `.`, since it is written into a `run:` line, and `plan` and `checks` are taken. The jobs check out the code and run the plan on `ubuntu-latest`; a step that needs a toolchain or dependencies the runner image lacks needs setup steps, and since `--write` replaces edits, such a workflow is better kept as your own.

For a workflow you wrote, `ci workflow` also prints the `if:` line for each gated job, to standard error when it prints the workflow:

```text
  if: needs.plan.outputs.run_e2e == 'true'   # e2e
  if: needs.plan.outputs.run_desktop_build == 'true'   # desktop-build
```

Each such job needs `needs: plan`, and the plan job must export the outputs (`run_e2e: ${{ steps.plan.outputs.run_e2e }}`) from a step that runs `fairlead ci plan --format github` itself: the action's outputs don't include `stage` or `run_<id>`.

**Required checks:** GitHub counts a job that `if:` skipped as passing a required status check. A required `e2e` check therefore passes on every pull request where end-to-end tests wait for the merge stage; it proves only that the job wasn't due. Require `checks` on pull requests, and watch the merge-stage jobs on the environment branch.

## `fairlead ci run --plan PATH`

Runs each invocation's argv in its working directory (relative to the repository root; a relative `--plan` is from the current directory), in order, without a shell, and exits 1 if any failed or has no argv, after running the rest. `--fail-fast` stops at the first failure. A plan of another version is refused with exit code 2.

*Since 0.7.0:* a program is found the way a shell finds it. On Windows, a name with no extension is looked up on `PATH` with each of `PATHEXT`'s `.com`, `.exe`, `.bat` and `.cmd`, and a relative path such as `node_modules/.bin/eslint` beside the working directory, so `npx` and other `.cmd` shims start. Before, they needed the full name, such as `npx.cmd`, which still works. The same goes for `fairlead done`, `[[guard.external]]` rules and graph providers.

An invocation a [`[[quarantine]]` entry](plan.md#tests-that-lie-on-one-platform) holds doesn't fail the run when its output matches the entry's signature and names no other failed test. It is reported as not provable here, and the last line counts it. Any other failure counts. The entry is checked again on the machine running the plan, so a plan made on Windows excuses nothing on Linux.

*Since 0.8.0:* `--only ID` runs just those runners and checks, and `--except ID` all but those, each repeatable, so one plan can feed a fast job and an end-to-end job. An id that no runner or check has fails with exit code 2. One the plan's stage defers is not an error: the run says which stage it waits for and runs nothing for it.

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
