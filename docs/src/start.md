# Start here: which project is yours?

*Since 0.20.0:* one command sets a repository up, and what it asks of you depends on the kind of repository it finds. [Install](install.md) Fairlead, then, at the repository's root:

```sh
fairlead setup
```

`setup` applies what is local and can be undone (a first config, the agent hooks, the git hook, the block in AGENTS.md and CLAUDE.md), proves each test runner by running one small test file, and prints a checklist. It never writes or edits a CI workflow, never deletes anything and never touches a `fairlead.toml` that already exists.

The checklist's first line names the **track** it detected. The first row that fits is yours:

| Your repository | Track |
|---|---|
| no test files, and under 20 commits | [new](#new) |
| over 180 days of history, and either a stack the test planner doesn't read or fewer than 1 test file for every 20 source files | [legacy](#legacy) |
| a workflow that runs tests or deploys, and over 50 test files or over 180 days of history | [ongoing](#ongoing) |
| anything else | [mvp](#mvp) |

The track decides which items are required and the order of the list. It never changes what a plan selects, it is never written to the config, and `fairlead setup --track NAME` names another for one run.

Every line of the checklist is a state, an item's id and what was found:

| State | Means |
|---|---|
| `ok` | nothing to do |
| `fixed` | `setup` changed it just now; commit what it wrote |
| `open` | yours to settle, with exactly one fix on the `fix:` line under it: one command or one config edit |
| `later` | not due yet, or not provable on this machine; the text says why |

`setup` exits 1 while a required item is open, else 0. `fairlead setup --check` prints the same list and changes nothing, `--dry-run` prints what `setup` would change, and `--json` prints the track, the items and the open count. In the samples below, the text after each id is what `setup` found in one repository, so yours will differ; the states and ids are the same. [Troubleshooting](troubleshooting.md) has an entry for each id.

## New

### Who it is for

A repository with no test files and under 20 commits: a scaffold, a prototype, the first week of a product.
There is nothing to plan yet, so the rules an agent works under come first.

### What you get on day one

- A `fairlead.toml` with [the guard only](install.md#the-guard-only): the [secret-path rule](guard.md#secret-paths), the [gitleaks preset](guard.md#gitleaks) and, where the tree holds migrations, a [migrations rule](guard.md#migrations).
- The hooks for each agent the repository shows: an edit that breaks a rule is refused as it is written, and a session doesn't end on a tree that hasn't passed the gate ([The write hook](guard.md#the-write-hook)).
- The git hook, which runs the same rules on what each commit adds.
- The block in AGENTS.md and CLAUDE.md that tells an agent the change loop ([AGENTS.md and CLAUDE.md](agents.md)).

### The command and the output to expect

```sh
fairlead setup
```

```text
fairlead setup    track: new (no tests yet, 6 commits)
  ok     binary     0.20.0 on PATH
  fixed  config     fairlead.toml, no problems, guard profile
  fixed  hooks      Claude Code: edit guard, brief nudge, stop gate, session start
  fixed  git-hook   the commit stage is in lefthook.yml, and lefthook runs it
  fixed  agents     the block is current in AGENTS.md, CLAUDE.md
  open   gate       no passing `fairlead done` on record
         fix: fairlead done
  ok     tests      no tests a runner here can run yet
  later  ci         no workflow runs `fairlead guard check`; the new track leaves it for later
1 open. Fix it and run `fairlead setup` again. `fairlead setup --check` shows this list any time.
```

The exit code is 0: `gate` is open but not required.

### What done looks like

`fairlead done` passes, which in this profile runs `fairlead guard check`, and `fairlead setup --check` shows no line that says `open`. Commit `fairlead.toml`, the hook settings, `lefthook.yml`, AGENTS.md and CLAUDE.md.

### What comes later

- **Tests.** Once the repository has test files a runner can run, the `tests` item opens. Its fix: `fairlead init --dry-run` prints the runner; add it to `fairlead.toml` and remove `profile = "guard"`. A repository with test files is no longer new: the next `fairlead setup` reads it as [mvp](#mvp), unless another track's row fits first.
- **CI**, once there is something for it to run: `fairlead ci workflow --write` ([Plans in CI](ci.md)).

### What to leave alone

[Replay](replay.md), [stages](ci.md#stages), `smoke` sets, the budget and the [scorecard](insights.md#score): each measures or tunes a test plan, and there is none yet. Don't write tests to move a checklist line; write them when the code needs them.

## MVP

### Who it is for

A repository with tests and a short history: a product on its way to its first users, a service a few months old.
CI may not exist yet, or may not run the tests.

### What you get on day one

- A `fairlead.toml` with a `[[tests.runners]]` entry for each runner the manifests show ([A first config](install.md#a-first-config-fairlead-init)), and every test file under exactly one runner.
- Proof that each runner starts: `setup` runs its smallest test file, with a 60 second limit.
- The agent hooks, the git hook and the AGENTS.md block, as on every track.
- The change loop: [`fairlead brief`](brief.md) before an edit, [`fairlead done`](done.md) before it's finished, running only the tests the change can reach.

### The command and the output to expect

```sh
fairlead setup
```

```text
fairlead setup    track: mvp (39 test files, no workflow that tests or deploys, 4 months of history)
  ok     binary     0.20.0 on PATH
  fixed  config     fairlead.toml, no problems
  ok     runners    1 runner, 39 test files, each with exactly one runner
  fixed  hooks      Claude Code: edit guard, brief nudge, stop gate, session start
  fixed  git-hook   the commit stage is in lefthook.yml, and lefthook runs it
  fixed  agents     the block is current in AGENTS.md, CLAUDE.md
  fixed  proved     1 runner ran one test file: vitest 0.0 s
  open   ci         no workflow runs `fairlead ci`
         fix: fairlead ci workflow --write
  ok     smoke      no runner has over 50 test files or a need
  open   gate       no passing `fairlead done` on record
         fix: fairlead done
  later  measure    the scorecard needs 10 more gate runs
2 open. Fix them and run `fairlead setup` again. `fairlead setup --check` shows this list any time.
```

The exit code is 0: on this track neither `ci` nor `gate` is required.

### What done looks like

`binary`, `config`, `runners`, `hooks`, `git-hook` and `agents` are `ok`, `proved` is `ok` or `later` with a reason you accept, and `fairlead done` has passed once, so `gate` is `ok`. `ci` may stay open until there is a CI to add it to.

### What comes later

- **CI:** `fairlead ci workflow --write` writes a workflow that plans and runs each pull request's tests ([A staged workflow](ci.md#a-staged-workflow-fairlead-ci-workflow)). Once a workflow runs tests or deploys, and the suite passes 50 files or the history 180 days, `setup` reads the repository as [ongoing](#ongoing), where `ci` is required.
- **A `smoke` set**, when a runner's suite passes 50 files or a [`[[needs]]` entry](done.md#needs) covers it: 1 to 3 files that stand for the suite ([The budget](done.md#the-budget)).
- **The scorecard**, after 10 gate runs: `fairlead insights --score` ([Score](insights.md#score)).

### What to leave alone

[Replay](replay.md), [stages](ci.md#stages), [sharding](ci.md#sharding-a-plan) and `done.budget_seconds`: the defaults suit a suite this size, and replay needs a CI history the repository doesn't have yet.

## Ongoing

### Who it is for

A repository whose CI already runs tests or deploys, with over 50 test files or over six months of history: a product people use.
Here the plan has to be proved in CI as well as on a laptop, so `ci` is required.

### What you get on day one

- Everything the [mvp](#mvp) track gets. A `fairlead.toml` that already exists is read and never changed, so `setup` also serves a repository that adopted an earlier release and wants to know what its install lacks.
- A gate that fits the machine: `fairlead done` runs the tests nearest the change within [the budget](done.md#the-budget) and records the rest as owed to CI.
- A checklist that fails, with exit code 1, until a workflow runs the plan.

### The command and the output to expect

```sh
fairlead setup
```

```text
fairlead setup    track: ongoing (412 test files, 2 workflows, 2 years of history)
  ok     binary     0.20.0 on PATH
  ok     config     fairlead.toml, no problems
  ok     runners    2 runners, 412 test files, each with exactly one runner
  fixed  hooks      Claude Code: edit guard, brief nudge, stop gate, session start
  fixed  git-hook   the commit stage is in lefthook.yml, and lefthook runs it
  fixed  agents     the block is current in AGENTS.md, CLAUDE.md
  later  proved     e2e can't run here: need "browser" not met
  open   ci         no workflow runs `fairlead ci`
         fix: fairlead ci workflow --write
  open   smoke      unit: 368 test files, and it names no `smoke`
         fix: name 1 to 3 test files under `smoke` on runner `unit` in fairlead.toml
  open   gate       no passing `fairlead done` on record
         fix: fairlead done
  later  measure    the scorecard needs 10 more gate runs
3 open. Fix them and run `fairlead setup` again. `fairlead setup --check` shows this list any time.
```

The exit code is 1: `ci` is open and required. `smoke` is open and not required.

### What done looks like

`fairlead setup --check` exits 0, a workflow runs `fairlead ci` or the action on every pull request, and the pull request carries the block `fairlead receipt --pr` prints, so what the gate left to CI is in front of the reviewer ([Claims and receipts](pr.md#claims-and-receipts)).

`ci workflow --write` writes `.github/workflows/fairlead.yml` and refuses to replace a workflow it didn't write. A repository that keeps its own workflow adds the plan to it instead ([Plans in CI](ci.md)); either one settles the item.

### What comes later

- **The scorecard**, after 10 gate runs: `fairlead insights --score` says how wide the plans are and what is owed to CI ([Score](insights.md#score)).
- **[Replay](replay.md)**, which measures the plan against the failures CI has recorded, before any test job is retired in the plan's favour.
- **[Stages](ci.md#stages)**, when a slow suite should wait for the merge, and **[`fairlead ci ledger`](ci.md#is-this-red-mine-fairlead-ci-ledger)**, when red checks need sorting into the change's own and the rest.

### What to leave alone

Your existing workflows: `setup` never edits one, and the jobs that run the whole suite stay until replay has shown the plan misses nothing. `plan.run_all` and `plan.ignore` keep their defaults until [insights](insights.md#suggestions) backs a change with evidence.

## Legacy

### Who it is for

A repository with over six months of history and few tests, under 1 test file for every 20 source files, or a stack the test planner doesn't read.
A test plan would select little here, so the guard is what earns its place first.

### What you get on day one

- Where the planner can't read the stack, a `fairlead.toml` with [the guard only](install.md#the-guard-only); where it can, the few runners the repository has, as on the [mvp](#mvp) track.
- Rules that judge a change by what it adds: the commit stage and the write hook fail on a finding the change introduces, never on what was already there ([Only what a change adds](guard.md#only-what-a-change-adds)). A file already tracked under a secret's name is a warning, not a failure ([Secret paths](guard.md#secret-paths)).
- Migrations that already exist can't be edited, moved or deleted, where the tree holds a migrations folder.
- The agent hooks, the git hook and the AGENTS.md block.

### The command and the output to expect

```sh
fairlead setup
```

```text
fairlead setup    track: legacy (6 years of history, the planner can't read the stack)
  ok     binary     0.20.0 on PATH
  fixed  config     fairlead.toml, no problems, guard profile
  fixed  hooks      Claude Code: edit guard, brief nudge, stop gate, session start
  open   git-hook   written but not active: lefthook.yml has the commit stage, .git/hooks doesn't run lefthook, and lefthook isn't on the PATH
         fix: install lefthook (it is what runs the commit stage), then `lefthook install`
  fixed  agents     the block is current in AGENTS.md, CLAUDE.md
  open   gate       no passing `fairlead done` on record
         fix: fairlead done
  ok     tests      no tests a runner here can run yet
  later  ci         no workflow runs `fairlead guard check`; the legacy track leaves it for later
2 open. Fix them and run `fairlead setup` again. `fairlead setup --check` shows this list any time.
```

The exit code is 0: `git-hook` is open, and it is required only on a machine that has `lefthook`, which is what runs the commit stage. `setup` writes the stage, and runs `lefthook install` itself when `lefthook` is on the PATH.

### What done looks like

`git-hook` is `ok`, `fairlead done` passes, and a commit that adds a secret-named file or edits an old migration is refused on every machine that clones the repository and has `lefthook`.

### What comes later

- **A size or comment rule**, one at a time: turn the preset on, then `fairlead guard check --write-baseline` records today's counts, so only a file that gets worse fails ([The ratchet](guard.md#the-ratchet)).
- **Runners**, as tests are written: the `tests` item opens when the guard-only repository has tests a runner can run.
- **CI:** `fairlead guard check` in a job with the guard only, or `fairlead ci workflow --write` once there is a plan.

### What to leave alone

The old code. Don't reformat it, don't turn on `findings = "all"`, and don't write a baseline for rules you haven't chosen. [Replay](replay.md) and the [scorecard](insights.md#score) measure a test plan and say little here.
