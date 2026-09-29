# The done gate

`fairlead done` runs what a change must pass before it counts as finished, and records the outcome against the exact tree it checked. An agent runs it before it stops; a person can run it before they push.

```bash
fairlead done                # plan the working tree against the default branch and run the gate
fairlead done --base main    # against another base
fairlead done --dry-run      # print the steps without running them
fairlead done --keep-going   # run every step even after one fails
fairlead done --check        # exit 0 only if this tree, as it stands, has passed
```

## The steps

In order:

1. The test invocations the change's plan selects, as `fairlead ci run` would run them.
2. The `[[checks]]` the plan selects.
3. The checks `done.always` names that the plan didn't select. Run outside a plan, a check's `{files}` stands for the change's files.
4. `fairlead guard check` over the whole tree, as CI runs it.

Each step prints its command, then whether it passed and how long it took. The gate stops at the first failure unless `--keep-going` is given; either way it passes only when every step passed.

```toml
[done]
tests  = "planned"      # planned | none
checks = "planned"      # planned | none
always = ["typecheck"]  # [[checks]] ids that run for every change
guard  = true
```

A check `done.always` names needs no `paths` or `modules`, since it always runs.

## The record

Every run appends a `done` event to `.git/fairlead/events.jsonl`: the plan's tree hash, whether it passed, and each step's id, outcome and seconds, never its output. A result belongs to one tree. The tree hash is HEAD's tree when the working tree is clean, else a hash of every file's path and blob id, so any edit after a pass makes it stale, and `--check` says so. Nothing needs clearing by hand, and switching branches or rebasing can't make an old result look current.

`[done]` never changes a plan, so it's left out of the plan's config digest, like `[guard]` and `[hooks]`.
