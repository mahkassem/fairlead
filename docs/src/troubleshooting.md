# Troubleshooting

*Since 0.20.0:* `fairlead setup --check` lists what a repository's setup lacks, and every open item names one fix ([Start here](start.md)). This page has one entry for each item id, in the checklist's order: the symptom as the checklist prints it, the cause, and the fix, for when the `fix:` line alone didn't settle it. `fairlead guide troubleshoot` prints the short form for the release that is installed.

```sh
fairlead setup --check    # the checklist; changes nothing
fairlead doctor           # the binary, the config, the hooks, the needs and the event log, in full
```

The text after an id is what was found in your repository, so it differs from the samples here; match an entry by its id. `fairlead doctor` ends with `setup: N open (first: <id>)`, and a session whose hooks are installed starts with `fairlead: setup has N open (<ids>)`, at most once a day: both point to the same list. In CI, `fairlead ci plan` warns when the checkout itself shows a gap (the config, the runners, the agents block, and the hooks where the agent's settings are committed): `setup has N open in the repository (<ids>)`.

| Id | Required | Checks |
|---|---|---|
| [`binary`](#binary) | yes | the binary that runs is the one the repository asks for |
| [`config`](#config) | yes | `fairlead.toml` loads |
| [`runners`](#runners) | yes | every test file has exactly one runner |
| [`proved`](#proved) | no | each runner ran one test file and it passed |
| [`hooks`](#hooks) | yes | the agent hooks are installed |
| [`git-hook`](#git-hook) | yes | the commit stage is written and active |
| [`agents`](#agents) | yes | the block in AGENTS.md and CLAUDE.md is current |
| [`ci`](#ci) | on the ongoing track | a workflow runs Fairlead |
| [`smoke`](#smoke) | no | a large suite names a smoke set |
| [`gate`](#gate) | no | `fairlead done` passed lately |
| [`measure`](#measure) | no | there are enough gate runs to score |
| [`tests`](#tests) | no | the guard profile only: tests a runner could run |

With `profile = "guard"`, `runners`, `proved`, `smoke` and `measure` are left out and `tests` is added.

## binary

**Symptom**

```text
  open   binary     the config needs 0.20 or later and this binary is 0.19.1
         fix: bun install
```

**Cause.** The binary that ran is older than the `fairlead` floor in `fairlead.toml`, or isn't the version `package.json` pins. The usual ways there: a dependency bump nobody installed, a new worktree whose `node_modules` is a copy of an older one, or a global install that comes first on the PATH.

**Fix.** Install the pinned release the way the repository installs it: its package manager's install, or the installer in [Install](install.md). `fairlead doctor` prints the path of the binary that runs and each version the repository names. For a new release rather than a stale install, follow [Upgrading](migrate.md#upgrade-in-five-steps).

## config

**Symptom**

```text
  open   config     no fairlead.toml; defaults apply
         fix: fairlead init
```

**Cause.** There is no `fairlead.toml`, or it loads with problems. The text is the first problem; an unknown key is an error, so a typo can't switch something off unnoticed ([Errors](config.md#errors)).

**Fix.** With no config, `fairlead init`, which `fairlead setup` runs for you. Otherwise `fairlead config check` lists every problem with its file and key; correct each key. `fairlead config show --origin` says which layer set a value. Don't delete the file to start over: `setup` never replaces a config, so the fix is an edit.

## runners

**Symptom**

```text
  open   runners    tests.runners `e2e`: its command names no `{files}`, so it runs its whole suite whenever any of its tests is selected
         fix: put `{files}` where the test files go in fairlead.toml
```

**Cause.** A test file has no runner, or more than one, or a runner's command names no test files, so it runs its whole suite whenever one of its tests is selected. The commonest case is an end-to-end spec named like a unit test, which the unit runner's `match` also takes.

**Fix.** Edit `[[tests.runners]]` in `fairlead.toml`: narrow a `match` or add an `exclude` until each test file has exactly one runner, and put `{files}` in the command. `fairlead config check` and `fairlead doctor` repeat the warning until it's gone ([A first config](install.md#a-first-config-fairlead-init)).

## proved

**Symptom**

```text
  open   proved     unit failed on src/money.test.ts: could not start (exit 1 before any test ran), with no output
         fix: install what the runner needs (the project's dependencies, or the service it talks to), then `fairlead setup`
  later  proved     e2e can't run here: need "browser" not met
```

**Cause.** `setup` ran each runner's smallest test file, with a 60 second limit. `open` means the test failed, or the runner couldn't start: its tool or the project's dependencies are missing. `later` means a [`[[needs]]` entry](done.md#needs) that covers the runner isn't met on this machine. `setup --check` runs nothing and reads the last result, kept in `.git/fairlead/setup.json`.

**Fix.** For a runner that couldn't start, install the project's dependencies and run `fairlead setup` again. For a failed test, run the runner's command on that file and read the error. A test that fails on a clean checkout is the repository's to fix; an agent reports it rather than editing the test to pass. An unmet need is not a fault: its `proved_in` says where those tests are proved.

## hooks

**Symptom**

```text
  open   hooks      no hooks for Claude Code
         fix: fairlead hooks install
```

**Cause.** An agent the repository shows has no Fairlead hooks, so nothing checks an edit as it is written and nothing asks for the gate before a session ends. The hooks live in a committed settings file by default; a repository that set `hooks.claude = "local"` has them per machine, and a new clone has none.

**Fix.** `fairlead hooks install`, with `--codex` or `--gemini` for those agents. `fairlead hooks status` says where each hook is ([The write hook](guard.md#the-write-hook)). Hooks from an older release are brought up to date by `fairlead migrate --write`.

## git-hook

**Symptom**

```text
  open   git-hook   written but not active: lefthook.yml has the commit stage, .git/hooks doesn't run lefthook
         fix: lefthook install
```

**Cause.** `lefthook.yml` has no commit stage, or has one that git never runs because lefthook hasn't put itself in `.git/hooks`. That folder isn't cloned, so every new clone and worktree starts without it.

**Fix.** `lefthook install`. With no commit stage, `fairlead hooks install --git` writes it first. With no `lefthook` on the PATH, install [lefthook](https://github.com/evilmartians/lefthook), as a dev dependency where the project has a package manager, then run both ([The git hook](guard.md#the-git-hook)).

## agents

**Symptom**

```text
  open   agents     AGENTS.md's fairlead block is stale
         fix: fairlead agents sync
```

**Cause.** A file in `agents.files` has no Fairlead block, or one that no longer matches the config, the always-on lessons and the routed skills.

**Fix.** `fairlead agents sync`, which writes only between its two markers. With `agents.write = "never"` the item is never open: a person owns those files, and `fairlead agents sync` prints the block to copy in ([AGENTS.md and CLAUDE.md](agents.md)).

## ci

**Symptom**

```text
  open   ci         no workflow runs `fairlead ci`
         fix: fairlead ci workflow --write
```

**Cause.** No workflow under `.github/workflows/` runs `fairlead ci` or the action, so what the gate owes to CI is never run there. The item is `later` on the new and legacy tracks, open but not required on mvp, and required on ongoing.

**Fix.** `fairlead ci workflow --write`, on a branch and through a pull request. It writes `.github/workflows/fairlead.yml` and refuses to replace a workflow it didn't write; where the repository keeps its own, add the plan to that one instead ([Plans in CI](ci.md)). `setup` itself never writes or edits a workflow.

## smoke

**Symptom**

```text
  open   smoke      e2e: 68 test files, and it names no `smoke`
         fix: name 1 to 3 test files under `smoke` on runner `e2e` in fairlead.toml
```

**Cause.** A runner whose suite is over 50 files, or which a `[[needs]]` entry covers, names no `smoke` set, or a `smoke` entry names no test file the runner matches, as after a rename (`fairlead config check` warns about that one too). When a change reaches such a suite only through a blanket and the suite doesn't fit the budget, the gate has nothing small to run in its place.

**Fix.** Under that runner in `fairlead.toml`, name 1 to 3 fast test files that stand for the suite, as `smoke = ["e2e/smoke.spec.ts"]` ([The budget](done.md#the-budget)).

## gate

**Symptom**

```text
  open   gate       no passing `fairlead done` on record
         fix: fairlead done
```

**Cause.** The event log on this machine holds no passing `fairlead done` from the last 30 days: the gate was never run here, or it has been failing.

**Fix.** `fairlead done`. When it fails, the line names the step and why ([The done gate](done.md#the-steps)); `fairlead done --check` says why a tree hasn't passed. The log is local, so a new clone starts with this item open.

## measure

**Symptom**

```text
  later  measure    the scorecard needs 7 more gate runs
```

**Cause.** The scorecard reads the gate's own record, and under 10 runs that carry what their plan reached say too little. This item is never open.

**Fix.** Nothing. Keep using the gate; after 10 runs, `fairlead insights --score` has something to say ([Score](insights.md#score)).

## tests

**Symptom**

```text
  open   tests      this repository has tests `vitest` can run, and the guard-only profile plans none
         fix: `fairlead init --dry-run` prints the runner; add it to fairlead.toml and remove `profile = "guard"`
```

**Cause.** The repository was set up with [the guard only](install.md#the-guard-only), and it now has test files a runner can run.

**Fix.** `fairlead init --dry-run` prints the config it would write now. Copy its `[[tests.runners]]` entries into `fairlead.toml`, remove `profile = "guard"`, and run `fairlead setup` again, which proves the runner.
