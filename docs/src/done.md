# The done gate

`fairlead done` runs what a change must pass before it counts as finished, and records the outcome against the exact tree it checked. An agent runs it before it stops; a person can run it before they push. *Since 0.8.0:* until it passes, [`fairlead next`](receipt.md#next) also says to open the pull request as a draft, `gh pr create --draft`; once it has, to mark it ready, `gh pr ready`.

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

*Since 0.7.0:* a test or check a [`[[quarantine]]` entry](plan.md#tests-that-lie-on-one-platform) holds on this machine is the exception. When it fails with the output its entry expects, the step is *not provable here*: it doesn't fail the gate, and the gate's last line says how many steps weren't provable, so the agent can say so in the pull request. Any other failure, or any failure after the entry's date, counts. This covers a `done.always` check too. A command in a step is found the way a shell finds it, so on Windows a `.cmd` shim such as `npm` or `pnpm` starts.

```toml
[done]
tests  = "planned"      # planned | none
checks = "planned"      # planned | none
always = ["typecheck"]  # [[checks]] ids that run for every change
guard  = true
```

A check `done.always` names needs no `paths` or `modules`, since it always runs.

## The record

Every run appends a `done` event to `.git/fairlead/events.jsonl`: the plan's tree hash, whether it passed, and each step's id, outcome and seconds, never its output. A step that wasn't provable here is recorded with `"quarantined": true`. A result belongs to one tree. The tree hash is HEAD's tree when the working tree is clean, else a hash of every file's path and blob id, so any edit after a pass makes it stale, and `--check` says so. Nothing needs clearing by hand, and switching branches or rebasing can't make an old result look current.

`[done]` never changes a plan, so it's left out of the plan's config digest, like `[guard]` and `[hooks]`.

## The Stop hook

`fairlead hooks install` also adds a Claude Code `Stop` hook, `fairlead guard stop`, unless `done.on_stop = "off"`. When the agent ends its turn, the hook checks whether the tree it leaves has passed `fairlead done`. It never runs the gate itself, which would outlast the hook's timeout; it reads the record.

- **Nothing changed** from the default branch's merge base: it lets the stop through, as for a session that only read.
- **A pass for this tree:** it lets the stop through.
- **Otherwise** it exits 2 with the reason on stderr, which Claude Code gives the agent instead of stopping: the gate hasn't passed for the tree as it is now, or its last run failed, and to run `fairlead done`.
- **`on_stop = "ask"`**, the default, sends the agent back once: when Claude Code says a Stop hook already did (`stop_hook_active`), the next stop goes through. **`"require"`** sends it back every time until the gate passes; Claude Code's own cap on consecutive blocks ends a loop.
- On any error, such as no base branch or an unreadable config, it lets the stop through. Each time it sends the agent back it records a `stop` event with the tree, never the reason's text.

```toml
[done]
on_stop = "ask"   # off | ask | require
```

