# The done gate

`fairlead done` runs what a change must pass before it counts as finished, and records the outcome against the exact tree it checked. An agent runs it before it stops; a person can run it before they push. *Since 0.8.0:* until it passes, [`fairlead next`](receipt.md#next) also says to open the pull request as a draft, `gh pr create --draft`; once it has, to mark it ready, `gh pr ready`.

```bash
fairlead done                # plan the change against its merge base and run the gate
fairlead done --base main    # against another base
fairlead done --dry-run      # print the steps without running them
fairlead done --keep-going   # run every step even after one fails
fairlead done --check        # exit 0 only if this tree, as it stands, has passed
fairlead done --pre-push     # the pre-push hook: the plan and the push checks
fairlead done --budget 300   # run what fits in 300 seconds and owe the rest to CI
fairlead done --full         # run the whole plan and every whole suite, with no budget
```

*Since 0.18.0:* the change is the branch as it differs from its merge base with `[pr] base`, else the remote's default branch, else `main` or `master`, with uncommitted and untracked files included, so commits already made count as much as edits not yet committed. With none of those branches, the gate says so in one line and plans what changed since HEAD ([What changed](plan.md#what-changed)).

## The steps

In order:

1. The test invocations the change's plan selects, as `fairlead ci run` would run them. *Since 0.19.0:* as many as fit [the budget](#the-budget), the tests nearest the change first.
2. The `[[checks]]` the plan selects.
3. The checks `done.always` names that the plan didn't select. Run outside a plan, a check's `{files}` stands for the change's files.
4. `fairlead guard check` over the whole tree, as CI runs it.
5. *Since 0.18.0:* `fairlead pr check --local`, when there are [`[pr]` rules](pr.md).

*Since 0.18.0:* with `profile = "guard"` ([The guard only](install.md#the-guard-only)) there's no plan, so the gate runs only steps 4 and 5. With a [`diff = true` external rule](guard.md#before-and-after), step 4 is `fairlead guard check --base` with the plan's merge base.

Each step prints its command, then whether it passed and how long it took. The gate stops at the first failure unless `--keep-going` is given; either way it passes only when every step passed. *Since 0.15.2:* a failed step also says why, read from its output, which is still printed as it comes: `could not start (exit 1 before any test ran)` when a runner's output shows no test result, such as a suite that needs a database that isn't up; `3 tests failed (exit 1)` when it names failed tests in a format Fairlead reads; otherwise the exit status after its tests ran; each with the first line that reads as an error:

```text
done: failed api in 0.5 s: could not start (exit 1 before any test ran): error: connect ECONNREFUSED 127.0.0.1:5432
```

*Since 0.7.0:* a test or check a [`[[quarantine]]` entry](plan.md#tests-that-lie-on-one-platform) holds on this machine is the exception. When it fails with the output its entry expects, the step is *not provable here*: it doesn't fail the gate, and the gate's last line says how many steps weren't provable, so the agent can say so in the pull request. Any other failure, or any failure after the entry's date, counts. This covers a `done.always` check too. A command in a step is found the way a shell finds it, so on Windows a `.cmd` shim such as `npm` or `pnpm` starts.

```toml
[done]
tests  = "planned"      # planned | none
checks = "planned"      # planned | none
always = ["typecheck"]  # [[checks]] ids that run for every change
guard  = true
budget_seconds = 300    # what fits runs here, the rest is owed to CI; 0 runs the whole plan
```

A check `done.always` names needs no `paths` or `modules`, since it always runs.

## The budget

*Since 0.19.0:* the gate runs what fits in `done.budget_seconds`, 300 by default, and records the rest of what the change reaches as debts for CI. So the gate can always pass on the machine it runs on, and nothing the change reaches is forgotten: it is owed, by runner and count, in the record, the receipt and the pull request.

The order, nearest the change first:

1. A test file the change edits, and canary tests. These always run, whatever the budget.
2. Tests that import a changed file.
3. Tests an [owner rule](plan.md#tests-and-classes) names for a changed path.
4. A runner's smoke set, when only a blanket reaches its suite (below).
5. Tests further along an import chain, shortest chain first.

A test fits when its runner's time for one file, added to what the run has already taken on, stays within the budget. That time is the median over the runner's recent passing steps in `done`'s own record, and the checks' median times count against the budget before any test does. A runner with no recorded time runs its nearest 20 files; that run records the time, so the next one fits the budget from it. *Since 0.20.1* files reached only through a blanket count among those 20, unless the runner names a `smoke` set or its whole suite waits for a later stage (`all_from`); until then such a first run ran none of them.

```text
done: unit: 41 of 380 reached test files run here; 339 owed to CI before merge: 339 past the 300 s budget
done: passed, 6 steps; owed to CI before merge: unit 339 test files (`fairlead receipt` lists them)
```

**A blanket** is what reaches a test when no file does: a `plan.run_all` path, the widening for a changed file nothing reaches, or `[[graph.edges]]` rules that reach `plan.blanket_share` (0.3) or more of a runner's suite. A suite reached only that way runs whole when all of it fits what's left of the budget. Otherwise the gate runs the runner's `smoke` set, the few files that stand for the suite, and owes the rest:

```toml
[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["npx", "playwright", "test", "{files}"]
smoke = ["e2e/smoke.spec.ts"]
```

```text
done: e2e: 1 of 44 reached test files run here; 43 owed to CI before merge: 43 reached only through a blanket (a `run_all` path, an unreached file or a wide `[[graph.edges]]` rule)
```

- **Debts are due** before merge, or by the nightly run in a project whose `phase` is `early`. A debt is paid by the CI that runs the plan on the pull request, as `fairlead ci run` does, and [`fairlead ci settle`](settle.md) is the check that fails a pull request whose debts never ran.
- **Under a budget the gate names the files it runs**, so a runner's `all_command` isn't used and no suite waits for a later stage: what doesn't fit is owed instead. `fairlead done --full`, `--budget 0` and `done.budget_seconds = 0` run the whole plan as before, with `all_command` and the waiting below.
- **An unmet [need](#needs)** leaves its files out before the budget is counted, as before.
- **[The Stop hook](#the-stop-hook)** reads the record as before. A budgeted pass is a pass, debts and all, so `on_stop = "require"` no longer asks an agent for a run its machine can't finish.
- **Advice** follows the steps when the plan backs it, each line starting `done: advice:`. Three kinds: a suite a blanket reaches whose runner names no `smoke` set; a new source file no test imports, directly or through other files (*since 0.20.0* a file named like a test is left out, wherever `tests.match` puts it); and a step that alone took longer than the whole budget. Advice never fails the gate. It is for the pull request's notes or a follow-up, and the gate says so.

*Since 0.15.0:* with no budget (`--full`, or `done.budget_seconds = 0`), a suite whose whole run waits for a later stage ([Whole suites](plan.md#whole-suites-at-a-later-stage)) runs only the tests the change reaches, and the gate prints one line for what it left out. When the change reaches most of such a suite, the gate passes the steps it ran but isn't finished: the record says `deferred`, `done --check` stays false and the Stop hook sends the agent back, until `fairlead done --full`, which runs every whole suite, passes on the same tree. In a project whose `phase` is `early`, nothing is deployed, so the gate passes and the scheduled run proves the suite. In `testing`, deferred holds until `done --full`; use `early` when nobody outside the team uses the build. *Since 0.15.2:* every line that reports a held suite names it, the phase that holds it, the share the change reaches, and the ways forward ([The project's phase](plan.md#the-projects-phase)); `done --check` says why a tree hasn't passed (not run on this tree, failed at a named step, or a suite waits) rather than only that it hasn't.

## CI history

*Since 0.18.0:* the gate runs first what CI has shown going red, when it's cheap, so the failure CI would report comes in seconds rather than after the slow suites. It reads two kinds of history, newest 20 runs in all:

- each ledger [`fairlead ci ledger`](ci.md#is-this-red-mine-fairlead-ci-ledger) cached in `.git/fairlead/ledger/`, as one run, counting only its failures judged `own`: a check by its job's name, a runner by its job's name or the failing test's file;
- each `fairlead ci run --results` file kept in `.git/fairlead/results/`, as one run, such as one downloaded from a workflow's artifacts.

A step is cheap when it says `cheap = true` (on a `[[tests.runners]]` or `[[checks]]` entry), or when its median time, from `done`'s own record and the results files, is at most `done.cheap_seconds` (60). A cheap step red on any of those runs runs first, the reddest first; a cheap check the plan didn't select is added, with `{files}` standing for the change's files. Each says why:

```text
done: typecheck: added, red in CI 6 of the last 20 runs
done: unit: first, red in CI 3 of the last 20 runs
```

A runner the plan didn't select isn't added, since that would run its whole suite. With no history, nothing changes.

## Before a push

*Since 0.18.0:* the pre-push hook `fairlead hooks install --git` adds runs `fairlead done --pre-push`: the plan, as `done` runs it, then the push checks (`fairlead guard check --push`: migration numbers against a freshly fetched base and the `[pr]` rules) in place of `pr check --local`. A tree that has already passed runs only the push checks, so a push after `fairlead done` costs seconds. It's installed when `[pr]` rules or `guard.migrations.fetch` ask for push checks, or when `done.pre_push = true`. `hooks install --git` moves a push entry an earlier release wrote, `fairlead guard check --push`, onto it.

## The record

Every run appends a `done` event to `.git/fairlead/events.jsonl`: the plan's tree hash, whether it passed, and each step's id, outcome and seconds, never its output. *Since 0.19.0:* and the budget it ran under, each debt (the runner, how many test files, why and when it's due), how many test files each runner step named, and the plan's reach: whether it selected everything, the test files it reached, the suite's size and how many ran here, which [`fairlead insights --score`](insights.md#score) reads. *Since 0.18.0:* and each step's CI stage (its `from`, or `ready` for a runner and `draft` for a check) and where it ran (`os`, `arch`), which [the receipt](receipt.md#stages-and-platforms) reads. A step that wasn't provable here is recorded with `"quarantined": true`. A result belongs to one tree. The tree hash is HEAD's tree when the working tree is clean, else a hash of every file's path and blob id, so any edit after a pass makes it stale, and `--check` says so. Nothing needs clearing by hand, and switching branches or rebasing can't make an old result look current.

`[done]` never changes a plan, so it's left out of the plan's config digest, like `[guard]` and `[hooks]`.

## The Stop hook

`fairlead hooks install` also adds a Claude Code `Stop` hook, `fairlead guard stop`, unless `done.on_stop = "off"`. When the agent ends its turn, the hook checks whether the tree it leaves has passed `fairlead done`. It never runs the gate itself, which would outlast the hook's timeout; it reads the record.

- **Nothing changed** from the default branch's merge base: it lets the stop through, as for a session that only read.
- *Since 0.15.2:* **The session can't have edited anything**: it lets the stop through, so a nested `claude -p` asked a question in the checkout, which loads the same hooks, answers once and isn't sent back. It reads the session's transcript (`transcript_path` in the hook's input): any use of `Edit`, `Write`, `MultiEdit`, `NotebookEdit`, `Bash` (a shell command can edit), `Task` or `Agent` (an agent it starts can) counts as a possible edit, and a line that doesn't parse is skipped. With no readable transcript it falls back to the write hook's events, which carry the session id: no edit logged since the newest passing `fairlead done` lets it stop. With `guard.events = "off"` as well, it can't tell and asks as before.
- **A pass for this tree:** it lets the stop through. *Since 0.18.0:* unless the pass left steps to where an unmet [need](#needs) is proved: then it sends the agent back once for that tree, whatever `on_stop` asks short of `off`, to say so in the pull request and keep it a draft until they pass there; the next stop goes through.
- **Otherwise** it exits 2 with the reason on stderr, which Claude Code gives the agent instead of stopping: the gate hasn't run on the tree as it is now, or its last run failed at a named step, or its steps passed while a whole suite waits (with what holds it and the ways forward), and the one thing to run next.
- **`on_stop = "ask"`**, the default, sends the agent back once: when Claude Code says a Stop hook already did (`stop_hook_active`), the next stop goes through. *Since 0.15.2:* and it sends a session back at most once for a given tree, read from the `stop` events it records with the session id, since Claude Code clears `stop_hook_active` at every new turn and the hook otherwise asked again on every turn of a long session. An edit makes a new tree, which it asks about once more. The reminder says it is the one for this tree. **`"require"`** sends it back every time until the gate passes; Claude Code's own cap on consecutive blocks ends a loop.
- On any error, such as no base branch or an unreadable config, it lets the stop through. Each time it sends the agent back it records a `stop` event with the tree and the session id, never the reason's text.

```toml
[done]
on_stop = "ask"   # off | ask | require
```

## Needs

*Since 0.15.0:* some tests need something the machine may not have ([#259](https://github.com/mahkassem/fairlead/issues/259), its first step), such as a mail catcher on a port, a database, a browser, or a container runtime for tests that start their own containers. A `[[needs]]` entry says what, which tests and checks need it, and how to check for it. Fairlead checks the need itself, never whether it runs in a container: a container can have every service beside it, and a laptop can have none.

```toml
[[needs]]
id = "smtp"
tcp = ["localhost:1025", "{env:MAIL_HOST}:1025"]  # met when any one accepts a connection, whatever answers
tests = ["tests/Feature/Mail/**"]
proved_in = "CI (services job)"
hint = "Start a local SMTP catcher on port 1025"

[[needs]]
id = "containers"
command = ["docker", "info"]                     # met when it exits 0
tests = ["tests/Integration/**"]

[[needs]]
id = "browser"
env = "BROWSER_PATH"                             # met when set and not empty
checks = ["e2e"]
```

- Each entry has exactly one of `tcp`, `command`, `env` and, *Since 0.18.0:*, `os`, and at least one of `tests` (globs) and `checks` (`[[checks]]` ids).
- *Since 0.18.0:* `os = ["macos"]` (or `windows`, `linux`) is met only on an OS it names, for what only that OS can build or run. The receipt lists what it left out as `unverifiable here: … needs macos`.
- In `tcp`, `{env:NAME}` is replaced by that variable, and an address naming an unset one is skipped. A lookup or connection takes at most `timeout_ms`, 300 by default; a `command` takes at most 2000 by default and is stopped after.
- A need that isn't met is tried once more after 200 ms, for a service that's still starting.
- On Windows, a connection to a closed port is tried again by the system for about a second before it's refused, so within the timeout it reads as `timed out` and costs the whole timeout. The need is missing either way.
- `[[needs]]` never changes a plan, so it's left out of the plan's config digest.

`fairlead doctor` checks every need at once and prints what it found:

```text
needs: 3 checked in 628 ms
  smtp: missing: localhost:1025 refused, {env:MAIL_HOST}:1025 skipped, MAIL_HOST unset (208 ms, 2 tries)
    to meet it: Start a local SMTP catcher on port 1025
    proved in: CI (services job)
  containers: missing: `docker info` exit 1 (628 ms, 2 tries)
  browser: met: BROWSER_PATH is set (0 ms)
```

A `command` need runs a command from the config, as `[[checks]]` do, so `doctor` runs it too.

*Since 0.15.2:* `fairlead done` reads `[[needs]]` ([#284](https://github.com/mahkassem/fairlead/issues/284), the first gate step of #259). It checks each need the plan's steps touch, once, the way `doctor` does, and when one isn't met:

- A test invocation that names its files runs without the files the need's `tests` cover, and is left out when none is left. One that runs its whole suite, naming no file, is left out only when the need covers every test it would run; otherwise it runs.
- A check the need's `checks` names is left out.
- With `proved_in`, what was left out counts as passed for `done`, `done --check`, `next` and the Stop hook, in any phase, since that place proves it before a merge. Without it, the gate doesn't pass, and says to meet the need or set `proved_in`.
- With `settle = true` under `[ci]`, what was left out is owed to CI instead, as a debt due before the merge, and the gate passes whether or not the need names `proved_in`: [the merge check](settle.md) fails the pull request when no job ran those files, so no text has to vouch for them. `fairlead config check` then warns about a `proved_in` nothing reads.
- The gate prints one line for each need and step, and the `done` event and the receipt record which need left what out of which step:

```text
done: api: 5 files not run here: need "postgres" not met (localhost:5432 refused); proved in CI (api job). To run them here: Start Postgres on 5432
done: db-lint: not run here: need "postgres" not met (localhost:5432 refused), and the need names no `proved_in`, so the gate can't pass. Meet it or set its `proved_in` to where these are proved, then run `fairlead done` again
```

Where the merge check collects them:

```text
done: api: 5 files not run here: need "postgres" not met (localhost:5432 refused); owed to CI, where the merge check collects it. To run them here: Start Postgres on 5432
done: passed, 4 steps; your change clean; environment: 1 skipped (postgres: owed to CI); owed to CI before merge: api 5 test files (`fairlead receipt` lists them)
```

*Since 0.18.0:* a pass that left steps to `proved_in` ends with the change's verdict apart from the machine's, so the agent can say both in the pull request:

```text
done: passed, 4 steps; your change clean; environment: 2 skipped (postgres: proved in CI (api job))
```
