# Fairlead in CI

Use this guide when `fairlead setup` prints `open   ci`, when you add Fairlead to a workflow, or when a check on your pull request is red and you need to know whether your change caused it.

## The commands, in order

```sh
fairlead ci workflow                # print a GitHub Actions workflow made from the config
fairlead ci workflow --write        # write it to .github/workflows/fairlead.yml
fairlead ci plan --format github    # in a job: plan the commit the job checked out
fairlead ci run --plan <path>       # in a job: run that plan's tests and checks
fairlead ci report --plan <path>    # in a job: the run's summary
fairlead ci settle --plan <path> --results <dir>   # in the last job: fail when a planned step has no green run
fairlead ci ledger                  # on your machine: judge each failure of HEAD's CI runs
```

## Adding it

- `ci workflow --write` replaces only a file it wrote, one that starts with its generated-file comment. It refuses any other, naming it, and never edits a workflow you wrote.
- A repository with its own workflow keeps it. Add a job that checks out with `fetch-depth: 0`, since the plan needs history for the merge base, then runs `ci plan` and `ci run --plan`. The workflow `fairlead ci workflow` prints is the shape to copy.
- With `profile = "guard"` there is no plan to run: the job runs `fairlead guard check`.
- A workflow is a change like any other: a branch and a pull request, never a push to the default branch.

## What the output means

- `ci plan` writes the plan as JSON outside the working tree. With `--format github` it sets step outputs: `plan` (the file's path), `tests` (how many test files), `checks` (the check ids), `all` (`true` when everything is selected).
- `ci plan` exits 2 when there is no merge base: the clone is shallow. Fetch the history; it never plans nothing instead.
- `ci run` exits non-zero when any invocation fails. `ci report` names each failed one with the command that runs it again.
- A debt `fairlead done` recorded, `owed to CI before merge`, is paid here: the job that runs the plan on the pull request runs those tests.
- `ci settle` is the check that collects those debts. It exits 1 when a step the commit's plan holds has no green run on that commit, and names the test files nobody ran; 2 when the plan is not this commit's or not whole. It never chooses tests again.

## The merge check

- Set `settle = true` under `[ci]`, then run `fairlead ci workflow --write`: each job writes its results and uploads them, and a last job named `settle` collects them. In a workflow you wrote, add that job by hand; the docs page "The merge check" has it.
- A person marks the `settle` job as a required check under the branch's protection rules. Don't change branch rules yourself; say that it is left to do.
- `not settled: e2e: no job ran it`: a job was skipped, cancelled or never started. Find why that job did not run; don't remove the step from the config to make the check pass.
- `attested by job X, not proven by results`: the config's `[ci.attested]` names a job for that step, and the job succeeded in this run. It is weaker than a results file, and the check says so each time.
- With the key on, what an unmet `[[needs]]` entry leaves out of `fairlead done` is owed to CI, and the gate passes without `proved_in`.

## Is this red mine?

```text
ledger 9f8e7d6  2 of 3 failures inherited: red on main since abc1234 (2026-11-07); 1 own
  inherited unit  test/b.test.ts > subtracts: red on main since abc1234 (2026-11-07)
  own       unit  test/a.test.ts > adds: not failing on main, and no rerun passed it
```

- `own`: nothing else explains it. Fix it in this change.
- `inherited`: the base branch fails the same way. Quote the line in the pull request; don't fix it in this change unless you were asked to.
- `flaky`: another attempt of the same commit passed. Rerun the job once. The ledger proposes a quarantine entry for a person to confirm; it writes nothing.
- `infra`: the job failed before any test ran, such as a lost runner or a failed install. Rerun the job.

`ci ledger` reads the GitHub API with `GITHUB_TOKEN`, `GH_TOKEN` or the token `gh auth token` prints, and never writes to GitHub. `fairlead ci ledger --rerun` reruns the failing test files that history can't explain, on HEAD and then on the merge base.

## What to do next

- After the workflow merges, `fairlead setup --check` shows `ok` for `ci`.
- Every failure is `own`: fix it, then `fairlead done` and push.
- None is `own`: say so in the pull request with the ledger's first line, and leave the base branch's failures to whoever owns them.
