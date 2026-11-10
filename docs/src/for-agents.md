# For agents

*Since 0.20.0:* this page is for a coding agent that has a shell and has never used Fairlead. It holds the six guides built into the binary, in the order a repository meets them. `fairlead guide` prints the same text for the installed release, so when the release you run isn't the one this book documents, read the command's output rather than this page.

```sh
fairlead guide            # the topics, one line each
fairlead guide setup      # one guide
```

| Topic | Read it when |
|---|---|
| [`setup`](#set-a-repository-up) | the repository has no `fairlead.toml`, or a session starts with `fairlead: setup has N open` |
| [`loop`](#the-change-loop) | you are about to make a change |
| [`gate`](#the-done-gate) | `fairlead done` fails or owes tests to CI, a hook refuses an edit, or you are writing the pull request |
| [`ci`](#fairlead-in-ci) | you add Fairlead to a workflow, or a check on the pull request is red |
| [`upgrade`](#upgrade-fairlead) | the repository moves to a new release |
| [`troubleshoot`](#troubleshoot-setup) | a setup item stays open after its fix |

Three things hold in every guide:

- **A setup item that is open has exactly one fix**, printed under it. Run that, then `fairlead setup` again.
- **A hook that refuses an edit names the rule to fix.** Change the edit; the refusal isn't something to get around.
- **A gate that owes tests to CI has passed.** Put the block `fairlead receipt --pr` prints in the pull request, and let CI run what is owed.

An unknown topic prints the list on standard error and exits 2. For a tool that reads an index rather than a book, [`llms.txt`](https://mahkassem.github.io/fairlead/llms.txt) at the site's root links every page with one line each.

## Set a repository up

Use this guide when a repository has no `fairlead.toml`, when a session starts with `fairlead: setup has N open`, or when `fairlead doctor` ends with `setup: N open`.

### The commands, in order

```sh
fairlead setup            # apply what is local and reversible, then print the checklist
fairlead setup --check    # print the checklist and change nothing
fairlead setup --dry-run  # print what setup would change
fairlead setup --json     # the checklist as JSON: the track, the items, how many are open
```

Run `fairlead setup` at the repository's root. It writes a first config when there is none, installs the hooks for each agent the repository shows, adds the git hook, syncs the block in AGENTS.md and CLAUDE.md, and runs each runner's smallest test file once to prove the runner starts. It never writes or edits a CI workflow, never deletes anything and never changes a `fairlead.toml` that already exists.

### What the output means

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

Each line is a state, an item's id and what was found.

- `ok`: nothing to do.
- `fixed`: setup changed it just now. Read `git status`, then commit what it wrote.
- `open`: yours to settle. Every open item has exactly one fix, on the `fix:` line under it: one command or one config edit. Do that and nothing else.
- `later`: not due yet, or not provable on this machine; the text says why. Leave it.

The exit code is 1 while a required item is open, else 0.

### The track

The first line names the track setup detected and why. The track decides which items are required. It is never written to the config.

- `new`: no tests yet and under 20 commits. The guard and the hooks; CI is `later`.
- `mvp`: some tests and a short history. The runners and the gate; CI is open but not required.
- `ongoing`: CI already runs tests or deploys. Every item, and CI is required.
- `legacy`: a long history with few tests, or a stack the test planner doesn't read. The guard first; CI is `later`.

When the track is wrong, name it for one run: `fairlead setup --track mvp` (or `new`, `ongoing`, `legacy`).

### What to do next

1. Apply each `fix:` line, top to bottom, then run `fairlead setup` again, until no line says `open`.
2. Commit what setup wrote on a branch: `fairlead.toml`, the agent's hook settings, `lefthook.yml`, AGENTS.md and CLAUDE.md.
3. A fix that doesn't settle its item: `fairlead guide troubleshoot` has an entry for each id.
4. Then make a change: `fairlead guide loop`.

## The change loop

Use this guide for every change in a repository that has a `fairlead.toml`: before the first edit, before you say the change is finished, and after.

### The commands, in order

```sh
fairlead brief <paths>   # before an edit: what the paths reach and what has to pass
fairlead done            # before you finish: the gate the change has to pass
fairlead receipt         # after it: the change against its brief, and the gate's state
fairlead next            # any time: the one step that's due
```

`<paths>` are the files or directories you expect to edit; a path that doesn't exist yet is a new file. A session has one brief: a later `fairlead brief` adds its paths to it.

### What the brief means

```text
brief b-b578d149  base 0346ca7  1 path
reaches  1 file imports them directly, 2 through the graph        graph importers, graph why
tests    1 test file                                              plan --files
checks   typecheck                                                plan --files
rules    size                                                     fairlead.toml [guard.*]
lessons  1 lesson                                                 .fairlead/lessons (fairlead lessons list)
skills   1 skill                                                  fairlead.toml [[skills.routes]]
done     unit, typecheck, guard                                   fairlead.toml [done], done --dry-run
next     edit, then: fairlead done
```

- `reaches`: the files that depend on the paths you named. A reach far wider than the task is a reason to stop and say so before editing.
- `tests`, `checks`: what will run for this change, and nothing else has to.
- `rules`: the guard presets that read these files. An edit that breaks one is refused as you write it.
- `lessons`: what earlier changes here taught. Read each one listed; `fairlead context <paths>` prints their text.
- `skills`: load the one named for the code you change.
- `done`: the gate's steps. The right-hand column is the command that checks each line.

### What the receipt and next mean

- `changed  3 files: 1 named in the brief, 1 in its reach, 1 outside`: an `outside` file is one the brief didn't count. Brief it (`fairlead brief <path>`) or say in the pull request why the change grew.
- `gate     passed for this tree`: the change is finished. `not run for this tree`: an edit came after the last pass, so run `fairlead done` again.
- `owed`: test files the gate left to CI. `fairlead guide gate` says what to do about them.
- `next`: do what the line says. It names `gh pr create --draft` until the gate passes and `gh pr ready` after.

### Also useful

```sh
fairlead find <words>            # search the lessons, skills, docs headings and declared names
fairlead test --explain <test>   # why a test is in the plan, or isn't
fairlead resume                  # a new session: where the last one on this branch stopped
fairlead learn --title "…" --path "src/x/**" --evidence <link> --source mistake   # record a lesson
```

### What to do next

- The gate failed, owes tests to CI, or a hook refused an edit: `fairlead guide gate`.
- A check is red on the pull request: `fairlead guide ci`.
- A session starts with `fairlead: setup has N open`: `fairlead guide setup`.

## The done gate

Use this guide before you say a change is finished, when `fairlead done` fails or prints `owed to CI`, when a hook refuses an edit, and when you write the pull request.

### The commands, in order

```sh
fairlead done            # run the gate on the change as it stands
fairlead done --check    # has this exact tree passed? exits 0 only if it has
fairlead receipt --pr    # the block for the pull request body
```

### The budget and debts

The gate runs what fits in `done.budget_seconds` (300 by default), nearest the change first: test files the change edits, then tests that import a changed file, then tests further along an import chain. What the change reaches and doesn't fit isn't dropped: it is recorded as a debt, owed to CI, by runner and count. So the gate can pass on any machine, and nothing the change reaches is forgotten.

### What the output means

```text
done: unit: 41 of 380 reached test files run here; 339 owed to CI before merge: 339 past the 300 s budget
done: passed, 6 steps; owed to CI before merge: unit 339 test files (`fairlead receipt` lists them)
```

- `done: passed, N steps`: this tree passed. Any later edit makes a new tree, so run the gate again after one.
- `owed to CI before merge`: a debt. The gate still passed, and the CI that runs the plan on the pull request pays it. Don't raise the budget or run `fairlead done --full` to clear it unless you were asked to. Put `fairlead receipt --pr` in the pull request, so a reviewer sees what ran here and what is owed.
- `done: failed <step> in 0.5 s: <why>`: the step's command and output are above the line. Fix the cause and run `fairlead done` again. `could not start (exit 1 before any test ran)` means the runner didn't start, such as a service that isn't up, not that a test failed.
- `N files not run here: need "postgres" not met … proved in CI`: this machine lacks something the tests need. With `proved in`, the gate passes and that place proves them; say so in the pull request. Without it the gate can't pass until the need is met.
- `done: advice: …`: never fails the gate. Advice is for the pull request's notes or a follow-up, not for this change, unless it is what you were asked to do.

### The pull request

```sh
fairlead done && fairlead receipt --pr
```

Paste the block into the body. Open the pull request as a draft, `gh pr create --draft`, and mark it ready, `gh pr ready`, once the gate passes. Write "verified" or "tests pass" only after the gate passed on the commit you push: `fairlead pr check` fails a body that claims it with no passing receipt for the head's tree.

### When a hook refuses an edit

The refusal lists findings as `file:line rule: message`. The rule it names is what to fix: change the edit so the finding goes, then write again. It is not something to bypass. Don't write the file through a shell command instead, don't commit with `--no-verify`, and don't turn the rule off: the commit stage and CI run the same rules. If the rule is wrong for this code, say so and let a person change `fairlead.toml`.

The Stop hook is the same kind of message: it sends you back while the tree you leave hasn't passed, and names the one command to run.

### What to do next

- Passed: `fairlead receipt`, then `fairlead next`.
- Failed twice at the same step for a reason outside your change: stop and report the step, its `<why>` and what you tried.
- A check is red on the pull request after a local pass: `fairlead guide ci`.

## Fairlead in CI

Use this guide when `fairlead setup` prints `open   ci`, when you add Fairlead to a workflow, or when a check on your pull request is red and you need to know whether your change caused it.

### The commands, in order

```sh
fairlead ci workflow                # print a GitHub Actions workflow made from the config
fairlead ci workflow --write        # write it to .github/workflows/fairlead.yml
fairlead ci plan --format github    # in a job: plan the commit the job checked out
fairlead ci run --plan <path>       # in a job: run that plan's tests and checks
fairlead ci report --plan <path>    # in a job: the run's summary
fairlead ci ledger                  # on your machine: judge each failure of HEAD's CI runs
```

### Adding it

- `ci workflow --write` replaces only a file it wrote, one that starts with its generated-file comment. It refuses any other, naming it, and never edits a workflow you wrote.
- A repository with its own workflow keeps it. Add a job that checks out with `fetch-depth: 0`, since the plan needs history for the merge base, then runs `ci plan` and `ci run --plan`. The workflow `fairlead ci workflow` prints is the shape to copy.
- With `profile = "guard"` there is no plan to run: the job runs `fairlead guard check`.
- A workflow is a change like any other: a branch and a pull request, never a push to the default branch.

### What the output means

- `ci plan` writes the plan as JSON outside the working tree. With `--format github` it sets step outputs: `plan` (the file's path), `tests` (how many test files), `checks` (the check ids), `all` (`true` when everything is selected).
- `ci plan` exits 2 when there is no merge base: the clone is shallow. Fetch the history; it never plans nothing instead.
- `ci run` exits non-zero when any invocation fails. `ci report` names each failed one with the command that runs it again.
- A debt `fairlead done` recorded, `owed to CI before merge`, is paid here: the job that runs the plan on the pull request runs those tests.

### Is this red mine?

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

### What to do next

- After the workflow merges, `fairlead setup --check` shows `ok` for `ci`.
- Every failure is `own`: fix it, then `fairlead done` and push.
- None is `own`: say so in the pull request with the ledger's first line, and leave the base branch's failures to whoever owns them.

## Upgrade Fairlead

Use this guide when `fairlead setup --check` prints `open   binary`, when `fairlead doctor` warns that `package.json` asks for a later version than the binary, or when you are asked to move a repository to a new release.

### The commands, in order

```sh
fairlead --version            # the release that runs now
bun add -d fairlead@latest    # or npm, pnpm, yarn: the way the repository installed the old one
fairlead migrate              # what would change; nothing is written
fairlead migrate --write      # write the changes
fairlead doctor               # the binary, the versions the repository names, the hooks
fairlead config check         # the config still loads
fairlead setup --check        # nothing is open that wasn't before
```

Install the new release the way the old one was installed. A package dependency moves with its package manager, which also updates `package.json` and the lockfile. Otherwise: the shell or PowerShell installer, `brew upgrade fairlead`, or `cargo install --locked fairlead`.

### What the output means

```text
migrate: from ^0.4.2 (package.json) to 0.7.0
  would update .claude/settings.json: Fairlead's hooks: adds the brief nudge and the Stop hook
  would update fairlead.toml: `fairlead = "0.4"` becomes "0.6": the config uses `done`, which 0.6 added
  review (0.5.0): `plan.ignore` applies to source files too: ...
migrate: dry run; `fairlead migrate --write` makes these 2 change(s)
```

- `from … to …`: the release the repository was on, and where migrate read it. When it guesses wrong, name it: `fairlead migrate --from 0.4.2`.
- `would update <file>`: a change `--write` makes. Migrate changes only Fairlead's own hook entries, the git hook's command, the config's version floor, the version in `package.json` and the action's pin in a workflow.
- `review (X.Y.Z)`: something that release changed that a person has to decide. Settle each one before `--write`. A decision that isn't yours goes in the pull request, quoted.
- After `--write`, when it moved the version in `package.json`, it names the command that updates the lockfile (`bun install`, `pnpm install`, `yarn install` or `npm install`). Run it; migrate never edits a lockfile.
- `fairlead migrate --check` writes nothing and exits 1 when anything would change, 0 when the repository matches the installed release, and 2 when the config doesn't load.

### What migrate never does

It never installs anything, never adds hooks to a repository that has none (`fairlead setup` does), never changes a setting that decides what a plan selects, and never moves a pin to an older release.

### What to do next

1. Commit the result on a branch: the manifest and its lockfile, the hook settings, `lefthook.yml`, `fairlead.toml` and any workflow whose pin moved.
2. Run `fairlead done`, then open the pull request with the `review` lines and what you decided for each.
3. `fairlead setup --check` shows a new open item: the release added it. Its `fix:` line says what to run; `fairlead guide troubleshoot` has the rest.
4. The upgrade notes for each release are in the book's Upgrading page, oldest first.

## Troubleshoot setup

Use this guide when `fairlead setup` or `fairlead setup --check` prints an `open` item and the `fix:` line under it didn't settle it. There is one entry per item id, in the checklist's order. After a fix, run `fairlead setup` again.

### binary
- Open when the running binary is older than the config's `fairlead` floor, or isn't the version `package.json` pins.
- Fix: install the pinned release the way the repository installs it (`bun install`, `npm ci`, the shell installer). Still open: another `fairlead` comes first on the PATH, and `fairlead doctor` prints the path of the one that runs.

### config
- Open when there is no `fairlead.toml`, or it loads with problems; the text is the first problem.
- Fix: with none, `fairlead init`. Otherwise `fairlead config check` lists every problem with its key; correct the key, and don't delete the file to start over.

### runners
- Open when a test file has no runner or more than one, or a runner's command names no `{files}`, so it runs its whole suite whenever one of its tests is selected.
- Fix: edit `[[tests.runners]]` in `fairlead.toml`: narrow a `match` or add an `exclude` until each test file has exactly one runner, and put `{files}` where the test files go in the command.

### proved
- Open when a runner ran its smallest test file and the test failed, or the runner couldn't start. `later` when a need that covers it is unmet here.
- Fix: run the runner's command on that file and read the error. A test that fails on a clean checkout is the repository's to fix: report it. A missing tool: install the dependencies (`npm ci`, `bun install`).

### hooks
- Open when an agent the repository shows has no Fairlead hooks.
- Fix: `fairlead hooks install`, with `--codex` or `--gemini` for those agents. `fairlead hooks status` says where each hook is.

### git-hook
- Open when `lefthook.yml` has no commit stage, or has one that is "written but not active": lefthook isn't in `.git/hooks`.
- Fix: `lefthook install`. No commit stage: `fairlead hooks install --git` first. No `lefthook` on the PATH: install it, then run both.

### agents
- Open when the Fairlead block in AGENTS.md or CLAUDE.md is missing or stale.
- Fix: `fairlead agents sync`. It writes only between its two markers. With `agents.write = "never"` the item is never open.

### ci
- Open when no workflow runs `fairlead ci` or the action. `later` on the `new` and `legacy` tracks.
- Fix: `fairlead ci workflow --write`, on a branch. It refuses to replace a workflow it didn't write; add a job to your own instead (`fairlead guide ci`).

### smoke
- Open when a runner whose suite is over 50 files, or which a `[[needs]]` entry covers, names no `smoke` set, or a `smoke` entry names no test file the runner matches. Not required.
- Fix: name 1 to 3 fast, broad test files under that runner's `smoke` in `fairlead.toml`.

### gate
- Open when no `fairlead done` has passed on this machine in the last 30 days. Not required.
- Fix: `fairlead done`. When it fails: `fairlead guide gate`.

### measure
- Never open. `later` until the event log holds 10 gate runs: "the scorecard needs N more gate runs". Nothing to fix; then read `fairlead insights --score`.

### tests
- Guard profile only. Open when the repository now has tests a runner can run. Not required.
- Fix: `fairlead init --dry-run` prints the runner. Add it to `fairlead.toml` and remove `profile = "guard"`.
