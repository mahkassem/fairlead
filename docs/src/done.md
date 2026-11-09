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

*Since 0.15.0:* a suite whose whole run waits for a later stage ([Whole suites](plan.md#whole-suites-at-a-later-stage)) runs only the tests the change reaches, and the gate prints one line for what it left out. When the change reaches most of such a suite, the gate passes the steps it ran but isn't finished: the record says `deferred`, `done --check` stays false and the Stop hook sends the agent back, until `fairlead done --full`, which runs every whole suite, passes on the same tree. In a project whose `phase` is `early`, nothing is deployed, so the gate passes and the scheduled run proves the suite. In `testing`, deferred holds until `done --full`; use `early` when nobody outside the team uses the build. *Unreleased:* every line that reports a held suite names it, the phase that holds it, the share the change reaches, and the ways forward ([The project's phase](plan.md#the-projects-phase)); `done --check` says why a tree hasn't passed (not run on this tree, failed at a named step, or a suite waits) rather than only that it hasn't.

## The record

Every run appends a `done` event to `.git/fairlead/events.jsonl`: the plan's tree hash, whether it passed, and each step's id, outcome and seconds, never its output. A step that wasn't provable here is recorded with `"quarantined": true`. A result belongs to one tree. The tree hash is HEAD's tree when the working tree is clean, else a hash of every file's path and blob id, so any edit after a pass makes it stale, and `--check` says so. Nothing needs clearing by hand, and switching branches or rebasing can't make an old result look current.

`[done]` never changes a plan, so it's left out of the plan's config digest, like `[guard]` and `[hooks]`.

## The Stop hook

`fairlead hooks install` also adds a Claude Code `Stop` hook, `fairlead guard stop`, unless `done.on_stop = "off"`. When the agent ends its turn, the hook checks whether the tree it leaves has passed `fairlead done`. It never runs the gate itself, which would outlast the hook's timeout; it reads the record.

- **Nothing changed** from the default branch's merge base: it lets the stop through, as for a session that only read.
- *Unreleased:* **The session edited nothing** since the newest passing `fairlead done`: it lets the stop through, so a nested `claude -p` asked a question in the checkout, which loads the same hooks, answers once and isn't sent back. It reads the write hook's events, which carry the session id; an edit with the `Edit`, `Write` or `MultiEdit` tool counts, an edit made with a shell command doesn't show there. With `guard.events = "off"` there are no such events, so it can't tell and asks as before.
- **A pass for this tree:** it lets the stop through.
- **Otherwise** it exits 2 with the reason on stderr, which Claude Code gives the agent instead of stopping: the gate hasn't run on the tree as it is now, or its last run failed at a named step, or its steps passed while a whole suite waits (with what holds it and the ways forward), and the one thing to run next.
- **`on_stop = "ask"`**, the default, sends the agent back once: when Claude Code says a Stop hook already did (`stop_hook_active`), the next stop goes through. *Unreleased:* and it sends a session back at most once for a given tree, read from the `stop` events it records with the session id, since Claude Code clears `stop_hook_active` at every new turn and the hook otherwise asked again on every turn of a long session. An edit makes a new tree, which it asks about once more. The reminder says it is the one for this tree. **`"require"`** sends it back every time until the gate passes; Claude Code's own cap on consecutive blocks ends a loop.
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
tcp = ["localhost:1025", "{env:MAIL_HOST}:1025"]  # met when any one accepts a connection
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

- Each entry has exactly one of `tcp`, `command` and `env`, and at least one of `tests` (globs) and `checks` (`[[checks]]` ids).
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

A `command` need runs a command from the config, as `[[checks]]` do, so `doctor` runs it too. The done gate doesn't read `[[needs]]` yet: a test that needs a missing service still fails `fairlead done`. What the gate will do with a need that isn't met is in #259.
