# The merge check

*Since 0.21.0.* `fairlead done` proves what fits its time budget on the machine where the work happens. Every other test the change
reaches is owed to CI. `fairlead ci settle` is the check that collects what is owed: it fails when a step the commit's
plan holds has no green run on that commit, and names the test files nobody ran.

```sh
fairlead ci settle --plan "$RUNNER_TEMP/fairlead-plan.json" --results results/
```

```text
fairlead: not settled: 1 step on 4f1c2ab with no green run
  e2e: no job ran it; 12 test files
    e2e/checkout.spec.ts
    e2e/sign-in.spec.ts
    and 10 more
  unit: 214 test files, proven by results
  lint: proven by results
```

It exits 0 when everything is settled, 1 when a step is not, and 2 when the plan is not one it can judge.

## What the gate owes

- The test files past the gate's time budget, and those a blanket rule reached beyond its smoke set
  ([Budget](done.md#the-budget)).
- With `settle = true` under `[ci]`, the tests and checks an unmet [`[[needs]]`](done.md#needs) entry left out. The
  gate passes without them and without `proved_in`, because this check fails the pull request when no job ran them.

The check does not read the gate's record. It holds CI to the commit's whole plan, which covers every debt a gate
on any machine could have left.

## What counts as a green run

A results file that `fairlead ci run --results` wrote, for the same plan, holding the step as passed. Give the check
each file, or a folder of them, such as the one the jobs' artifacts were downloaded into.

- A step the plan holds that no results file names is not settled. A job that an `if:` skipped, a shard that never
  started and a step left behind by `--fail-fast` all end here. Without this check they pass silently.
- For a sharded plan, every shard has to report. A run of the whole step, with no `--shard`, covers all of its shards.
- When a job runs again, the newer attempt replaces the older one: a pass does not outlive a later failure of the same
  step, and a failure is forgiven by a later pass.
- A failure that a `[[quarantine]]` entry expects where it ran is not a green run. The step needs one somewhere on that
  commit, such as the job on the platform where it passes.
- Results for another plan are left out, and so count for nothing.

## A job that does not go through `fairlead ci run`

Some suites run in a job of their own, with their own command. The stronger way to bring one in is to have that job
run its step through Fairlead, `fairlead ci run --only e2e --results ...`, and upload the file. Where that is not
possible, name the job:

```toml
[ci.attested]
e2e = ["browser"]
api = ["api", "api-reach"]
```

A step with no results file is then taken as attested when every job named for it succeeded in the same workflow
run. A matrix's legs, `browser (1/2)` and `browser (2/2)`, are all the job `browser`, and each has to succeed. A job
that was skipped, cancelled, still running or absent from the run settles nothing, so the settle job has to `needs:`
the jobs it reads, and they have to be in the same workflow.

This is weaker than a results file: a green job says the job ran, not which test files it ran. The check says so
every time, apart from what results prove:

```text
  e2e: 12 test files, attested by job browser, not proven by results
  unit: 214 test files, proven by results
```

The job needs a token that can read the run's jobs (`actions: read`, with `GITHUB_TOKEN` in its environment).
Without one nothing is read and nothing is taken on trust.

## The plan has to be this commit's, and whole

The check does not choose tests again. It takes the plan the run already made and refuses one that could settle green
without looking at the whole change:

- a plan for another commit, or one made on uncommitted changes;
- a plan made with `--files`, which names no base commit;
- on a pull request or a merge queue, a plan whose base is not the change's base or behind it;
- a plan for an earlier stage than the event is, such as a draft's plan on a pull request that is ready.

A plan of everything is accepted wherever it starts.

## What waits

A step that the stage leaves for a later one is not due yet. The check lists it and says which run has to collect it:

```text
fairlead: waiting, not due at this stage:
  e2e: 12 test files, due after the merge, on the push run
  api: 41 test files, due at the nightly run
```

A pull request is checked at the ready stage. A step with `from = "merge"` is collected by the run on the push that
follows the merge, and a suite with `all_from = "full"` by the nightly run, where the plan is everything and every
step in it has to be green.

When something waits for the nightly run and the job can read the workflow's runs, the check adds how the last one
ended, since a red night collected nothing:

```text
fairlead: the last nightly run, on 2026-03-03 at 9c1e0aa, ended failure: what waits for the nightly run is not collected until one is green
```

## In a workflow

With `settle = true` under `[ci]`, `fairlead ci workflow --write` does this for the workflow it generates: each job
writes its results and uploads them, a last job named `settle` collects them, and the workflow also runs on a merge
queue, where a required check has to report or the queue waits forever. `fairlead setup` then leaves its `ci` item
open for as long as no workflow runs the check.

In a workflow of your own, every job that runs part of the plan writes a results file and uploads it. One last job
downloads them and settles:

```yaml
settle:
  needs: [plan, checks, e2e]
  if: always()
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v7
      with:
        fetch-depth: 0
    - uses: actions/download-artifact@v8
      with:
        path: results
    - run: >
        fairlead ci settle --plan results/fairlead-plan/fairlead-plan.json --results results
        --needs '${{ toJSON(needs) }}' --format github
```

- `if: always()` matters. GitHub counts a skipped job as passing a required check, so a settle job that only runs when
  the others succeed says nothing when one of them fails.
- `--needs` lets the check see a cancelled run: a job GitHub cancelled never finished, so nothing it left counts and
  the check fails.
- `fetch-depth: 0` gives it the history it needs to compare the plan's base with the change's.
- Mark the job as a required check under the branch's protection rules. A check nobody requires is advice.

`--out` writes the record as JSON: what is settled and by what, what is not and why, and what waits. The checkout
keeps the newest twenty under its git folder.
