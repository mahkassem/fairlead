# Upgrading

*Since 0.7.0:* `fairlead migrate` brings a repository to the release you
just installed. It updates what an older release left behind: the hooks
`hooks install` wrote, the config's `fairlead` version floor, and the
version pins in `package.json` and the workflows. It then lists the changes
since your release that need a person to decide.

Each step looks at what the repository has, not at which version it came
from. So one run catches up from any older release, and a second run finds
nothing to do.

## Upgrade in five steps

1. Install the new release the way you installed the old one:

   ```sh
   bun add -d fairlead@latest      # or npm, pnpm, yarn; this updates package.json and the lockfile
   # or the shell or PowerShell installer from Install
   ```

2. See what would change. Nothing is written yet:

   ```sh
   fairlead migrate
   ```

   ```text
   migrate: from ^0.4.2 (package.json) to 0.7.0
     would update .claude/settings.json: Fairlead's hooks: adds the brief nudge and the Stop hook
     would update lefthook.yml: the commit stage runs `bun x fairlead guard check --staged` instead of `fairlead guard check --staged`
     would update fairlead.toml: `fairlead = "0.4"` becomes "0.6": the config uses `done`, which 0.6 added
     would update .github/workflows/ci.yml: Fairlead's action at v0.4.2 becomes v0.8.0
     review (0.5.0): `plan.ignore` applies to source files too: ...
     review (0.6.0): `hooks install` adds a Stop hook, ...
   migrate: dry run; `fairlead migrate --write` makes these 4 change(s)
   ```

3. Read the `review` lines. They come from [Release by release](#release-by-release)
   below, filtered to the releases since yours and to what applies to this
   repository. Settle each one before you write: for example, set
   `brief.nudge = false` first if you don't want the nudge added.

4. Write the changes:

   ```sh
   fairlead migrate --write
   ```

   If it moved Fairlead's version in `package.json`, it says which command
   updates the lockfile (`bun install`, `pnpm install`, `yarn install` or
   `npm install`). Run that command; migrate never edits a lockfile.

5. Check the result and commit it:

   ```sh
   fairlead doctor
   fairlead config check
   fairlead migrate --check    # exits 0 once nothing is left
   ```

`--from VERSION` names the release you came from, when migrate can't tell
or guesses wrong. It reads the release from `package.json`, then from a
workflow's pin, then from the config's floor.

## What migrate changes

| What | When it changes | How |
|---|---|---|
| Claude Code hooks in `.claude/settings.json` and `settings.local.json`, and, since 0.9.0, Codex hooks in `.codex/hooks.json` and Gemini CLI hooks in `.gemini/settings.json` | Fairlead's entries aren't the ones this release's `hooks install` would write for your config | Fairlead's entries are replaced with this release's, and every other setting and hook stays as it was. Uninstall still restores the file from before Fairlead, byte for byte, when nobody else changed it. |
| The commit stage in `lefthook.yml`, and *Since 0.18.0:* the push entry | It runs another command than this release writes, such as `fairlead` by name where the project has its own copy, or `fairlead guard check --push` where this release runs `fairlead done --pre-push` | The `run:` line is replaced in place, with its indentation, quotes and comments kept |
| The `fairlead = "X.Y"` floor in the project config | The floor is older than a table or key the config uses, such as `[done]` (0.6) or `[guard]` (0.4) | The floor is raised to that release, so a teammate on an older binary is told which version to install |
| `package.json` | `dependencies`, `devDependencies` or `optionalDependencies` pins an older release, as `0.4.2`, `^0.4.2`, `~0.4.2` or `>=0.4.2` | The version moves to this release and keeps its `^` or `~` |
| Workflows under `.github/workflows/` | A step uses `mahkassem/fairlead@vX.Y.Z`, or passes `version: vX.Y.Z`, for an older release | Both move to this release |
| *Since 0.11.0:* `.github/workflows/fairlead-insights.yml` | `ci workflow --insights` wrote it (it starts with the generated-file marker) and this release writes it differently, as 0.11.0 does with the scopes `replay fetch` needs | It's rewritten as `ci workflow --insights --write` would. A file without the marker is the team's own and is left alone. |

What migrate never does:

- It never installs anything or runs a package manager, and never edits a lockfile.
- It never adds hooks to a repository that has none; that's `hooks install`.
- It never adds a floor to a config that has none, never changes a setting
  that decides what a plan selects, and never moves a pin to an older release.
- It leaves pins by branch, by commit or `latest` alone.

## In CI

`fairlead migrate --check` writes nothing and exits 1 when anything would
change, so a dependency bot's version bump can't be merged half done:

```yaml
- run: npx fairlead migrate --check
```

It exits 0 when the repository matches the installed release, and 2 when
the config doesn't load or a file can't be read.

## Release by release

Every release with something to do, oldest first. Plan ids change with
every release, so a plan made by one release isn't reused by another.

### To 0.4.0 (from 0.3.0)

- `[[guard.external]]` refuses `stages = ["write"]`, because the file isn't
  written yet when the write hook runs. Give such a rule `commit` or `ci`.
- A plan's config digest leaves out `guard` and `hooks`, so changing only
  those keeps plan ids. Nothing to do.

### To 0.5.0

Migrate updates:

- The hooks of a repository that lists Fairlead as a package dependency, so
  they run the project's own copy instead of `fairlead` on the `PATH`.

For you to decide:

- `plan.ignore` applies to source files too. A changed source that nothing
  imports and that matches it now selects nothing, where it used to widen to
  every test. Under the defaults that's `docs/**`.
- An owner rule whose `match` names no test file no longer counts as
  covering its paths, so they fall to `tests.unreached`. `config check` warns
  about each one; a rule meant to select nothing should be a `plan.ignore`
  entry instead.
- `key = []` over a list that already has items changes nothing, because
  lists append. `{ replace = [] }` clears it, and `config check` warns where
  a layer does this.
- A Go module's `go.mod` and `go.sum` are graph edges instead of
  `plan.run_all` defaults, so a dependency bump selects that module and its
  importers. If your config lists them in `plan.run_all` itself, it still
  runs everything on them; drop them to get the narrower plan.
- Go, Python, PHP, Vue, Svelte and Astro files are in the import graph. A
  repository that ran a whole suite through a `[[checks]]` entry can give
  its tests a runner instead ([Import graph](graph.md)).
- Replay datasets fetched before 0.5.0 keep no lines for a bun test file
  that failed to load. Fetch them again to count those failures.

### To 0.5.1

- A changed path that matches `plan.ignore` and reaches no test is ignored
  even when other files import it. In 0.5.0 it went to `tests.unreached`.
  Plans for such changes get smaller; nothing to do.

### To 0.6.0

Migrate updates:

- Hooks installed before 0.6.0 get the Stop hook and the brief nudge, which
  `hooks install` adds since 0.6.0. `hooks install` itself leaves existing
  hooks alone, so a repository that only re-ran it kept the old set.

For you to decide:

- The Stop hook sends an agent back while `fairlead done` hasn't passed for
  the tree it leaves ([The done gate](done.md)). Set `done.on_stop = "off"`
  before `migrate --write` to leave it out, or `"require"` to ask every time.
- The nudge notes an edit made with no brief ([The brief](brief.md)). Set
  `brief.nudge = false` to leave it out.

### To 0.7.0

Migrate updates:

- The config's `fairlead` floor, to 0.7, where the config uses `[[quarantine]]`.

For you to decide:

- Java and Kotlin files are in the import graph. A Maven or Gradle build that
  ran everything through a `[[checks]]` entry can give its tests a runner
  instead, with `{class}` or `{classes}` in its command
  ([Java and Kotlin](graph.md#java-and-kotlin)).
- `fairlead migrate` refreshes Claude Code's hooks. Hooks for Codex
  (`hooks install --codex`) and Gemini CLI (`hooks install --gemini`) are new
  in 0.7.0, so there's nothing older to bring along.

### To 0.7.1

Migrate updates:

- The config's `fairlead` floor, to 0.7.1, where a runner uses `all_command`
  or `exclude_arg`.

For you to decide:

- A runner whose tool, given no files, finds more tests than its `match`
  claims (a bare `bun test` that also finds end-to-end specs) can set
  `all_command` for the run of everything. A runner holding a test in
  `[[quarantine]]` can set `exclude_arg` so that run stays whole
  ([Configuration](config.md)).

### To 0.8.0

Migrate updates:

- Claude Code hooks installed before 0.8.0 get `Skill` in the edit hook's
  matcher, so a skill loaded through the `Skill` tool counts as a use, and a
  `SessionStart` hook that runs `fairlead resume --hook`. Set
  `brief.resume = false` before `migrate --write` to leave that one out
  ([Context and resume](context.md)).
- The config's `fairlead` floor, to 0.8, where the config uses `[memory]`,
  `[skills]`, `[agents]` or `brief.resume`.

For you to decide:

- Lessons, skill routes and the AGENTS.md block stay off until a repository
  adds them. `fairlead import lessons` turns a lessons document a team
  already keeps into lesson files, and `fairlead import rules` turns
  path-scoped rule files into routed skills ([Lessons](lessons.md),
  [Skills](skills.md)).
- CI stages stay off until the config sets `[stages]` or a runner's or
  check's `from`. Then `ci plan` reads the stage from the GitHub event, a
  merge skips what its pull request passed on the same tree, and
  `fairlead ci workflow` writes a staged workflow or prints the `if:` lines
  for one you keep ([Stages](ci.md#stages)).
- Three kinds of change no longer select everything: a bun dependency's
  version bump runs the tests of the files that import it, an edit to
  `fairlead.toml` plans with the new config, and a workflow that only runs by
  hand or on a schedule selects nothing by itself ([The test plan](plan.md)).
- `migrate` refreshes Claude Code's hooks only. Gemini CLI hooks installed
  by 0.7 don't match `read_file`, so reading a SKILL.md there isn't counted
  as a use until `fairlead hooks uninstall --gemini` and
  `hooks install --gemini` write this release's.

### To 0.9.0

Migrate updates:

- Codex hooks in `.codex/hooks.json` and Gemini CLI hooks in
  `.gemini/settings.json`, which `migrate` now refreshes as it does Claude
  Code's. Gemini CLI hooks from 0.7 get `read_file` in the nudge's matcher,
  so a SKILL.md read there counts as a use; the uninstall and install the
  0.8.0 notes asked for is no longer needed.

For you to decide:

- A project whose pushes run the plan and whose full suite runs on a
  schedule can add `--event schedule` to its `replay fetch`, so replay
  counts what the nightly caught and no push ran ([Scheduled
  runs](replay.md#scheduled-runs)).
- `fairlead graph why` names edge kinds in words (`type import`,
  `path literal`) where it printed `typeimport` and `pathliteral`; a script
  that reads its output needs the new names.

### To 0.10.0

Migrate updates nothing in your files: every new table is optional.

For you to decide:

- Replay sets recurring failures aside: a test failing in one job across
  three or more unrelated pull requests gets the outcome `recurring` and
  leaves adjusted recall, so a gate can pass that failed before. Confirm
  the `[[replay.quarantine]]` entries it lists ([Recurring
  failures](replay.md#recurring-failures)).
- `fairlead doctor --score` rates the setup out of 100 and names the three
  fixes worth the most ([Readiness score](score.md)); `fairlead ci workflow
  --insights --write` adds the weekly summary ([Insights](insights.md)).
- `fairlead mcp` serves read-only tools to an agent ([MCP server](mcp.md)),
  `[tracker]` puts a branch's task in the brief ([Trackers](trackers.md)),
  and `ci.escapes = "file"` opens an issue per escape ([CI](ci.md)).

### To 0.11.0

Migrate updates:

- `.github/workflows/fairlead-insights.yml`, when `ci workflow --insights`
  wrote it: it's rewritten with the `pull-requests` and `checks` read scopes
  `replay fetch` needs ([#201](https://github.com/mahkassem/fairlead/issues/201)).

For you to decide:

- Sessions in a linked worktree count now: every worktree writes to the
  main checkout's event log, so `doctor --score`, `insights` and
  `tokens report` may show more than before.
- A Next.js app can add `extends = ["next"]`, which `fairlead init` now
  names ([Framework packs](graph.md#framework-packs)).
- A workspace can declare `[[workspace.edges]]` between its repositories
  ([Edges across repositories](workspace.md#edges-across-repositories)).
- Building from source needs Rust 1.95.

### To 0.12.0

Migrate updates nothing in your files.

For you to decide:

- The `next` pack's end-to-end rule, every spec that calls `page.goto(`
  reaching every route and `middleware`, is its own pack now. Add
  `"next-e2e"` to `extends` to keep it, or leave it out when your owner
  rules already map pages to specs
  ([Framework packs](graph.md#framework-packs), [#212](https://github.com/mahkassem/fairlead/issues/212)).
- With more than one runner, a runner whose folder holds another runner's
  tests names its own when everything runs, so its command line lists
  them; `all_command` keeps the tool's own discovery
  ([Planning](plan.md), [#210](https://github.com/mahkassem/fairlead/issues/210)).
- Under Bun, `"trustedDependencies": ["fairlead"]` has `bun install`
  download the binary instead of the first command ([Install](install.md)).

### To 0.13.0

Migrate updates:

- The weekly insights workflow that `ci workflow --insights` wrote, which
  called `replay run` without its required `--config` and so failed its
  replay step on every run.

For you to decide:

- A changed `graph.barrier` file selects the tests of the files that import
  it; a change below it still stops at it. A `plan.run_all` entry that
  covered only a barrier's own changes can go, and `fairlead migrate` names
  the case when an entry is a barrier entry or a path a barrier matches
  ([Barriers](graph.md#barriers), [#225](https://github.com/mahkassem/fairlead/issues/225)).
- A `plan.run_all` entry can name the kinds of change that run everything,
  so a new migration can go through the graph while an edited one still runs
  everything. A config that uses `on` needs 0.13 to load
  ([Run everything by kind of change](plan.md#run-everything-by-kind-of-change)).
- On Alpine and other musl systems the npm package now installs the musl
  build, and on a platform with no build it warns instead of failing the
  install, so `--ignore-scripts` is no longer needed in such images
  ([Install](install.md), [#218](https://github.com/mahkassem/fairlead/issues/218)).

### To 0.14.0

Migrate updates nothing in your files.

For you to decide:

- A named import through a barrel (`import { a } from "./lib"`) now depends on
  the file that declares `a` and the barrels on the way, not on everything the
  barrel re-exports, so plans get narrower and `graph why` and `test --explain`
  print `re-export` for those steps. A barrel with a module behind it that runs
  code when it loads keeps its importers on the whole barrel; code inside a
  top-level initializer isn't seen, and `graph.reexports = false` turns this
  off ([Named imports through re-exports](graph.md#named-imports-through-re-exports),
  [#221](https://github.com/mahkassem/fairlead/issues/221)).
- On Bun 1.4.4 or later, `fairlead test bench` times serial `bun test`,
  `--parallel` and `--parallel --no-isolate` on your tests and says which to
  run; `fairlead migrate` names a runner still on serial `bun test`, and
  `doctor --score` gains a `bench` item for repositories with one
  ([Measuring Bun's test modes](plan.md#measuring-buns-test-modes),
  [#237](https://github.com/mahkassem/fairlead/issues/237)).
- `init` on Bun 1.4.4 writes the parallel runner and offers a `bun-fmt` check
  on the changed files; an existing config keeps its runner.
- Fairlead installs with Composer, and from the next release with
  `cargo install fairlead` and a Homebrew tap ([Install](install.md)).

### To 0.14.1

Migrate updates nothing in your files. 0.14.1 is 0.14.0 with its release
fixed: 0.14.0 reached crates.io but has no GitHub release, npm package or
Homebrew formula, so upgrade from 0.13 straight to 0.14.1 and read
[To 0.14.0](#to-0140) for what changed.

### To 0.14.2

Migrate updates nothing in your files. 0.14.2 fixes the npm install on
Windows when a path has a space in it.

### To 0.15.0

Migrate updates nothing in your files. 0.15.0 adds `[[needs]]`, which
`fairlead doctor` checks; the done gate doesn't read it yet
([Needs](done.md#needs)).

0.15.0 also adds `all_from` on runners and checks and the top-level
`phase`, which leave slow suites to a scheduled run. A config that sets
neither plans exactly as before. To use them, set `phase = "early"` while
nothing is deployed, or `all_from = "full"` on a slow runner, then run
`fairlead ci workflow --write` again so the workflow gets its schedule
([Whole suites at a later stage](plan.md#whole-suites-at-a-later-stage)).

### To 0.15.1

Migrate updates nothing in your files. 0.15.1 fixes `fairlead doctor`'s
phase line, which missed a production deploy whose workflow names the
environment on its own line (`environment:` then `name: production`) and so
suggested `phase = "early"` for a live repository.

### To 0.15.2

Migrate updates nothing in your files. 0.15.2 makes `fairlead done` read
`[[needs]]`: a need that isn't met leaves out the tests and checks it
covers, and the gate passes without them only when the need names
`proved_in`. Give each need whose tests CI proves a `proved_in`, or the
gate stays open on a machine without it ([Needs](done.md#needs)).

The Stop hook asks a session once per tree under `done.on_stop = "ask"`,
and lets a session whose transcript shows no tool use that can change
files stop; with no transcript it reads the write hook's events, and with
`guard.events = "off"` as well it keeps the old behaviour
([The Stop hook](done.md#the-stop-hook)). An untracked folder
holding a `CACHEDIR.TAG` stays out of plans and the tree hash, and an edit
to `fairlead.toml` no longer reaches tests through a file that names it.

### To 0.18.0

0.18.0 also carries what was planned as 0.16 and 0.17; there is no 0.16.0
or 0.17.0. `fairlead migrate --write` moves the floor, and moves a lefthook
`guard check --push` entry onto `fairlead done --pre-push`.

- **The secret-path rule is on with no config.** A commit may not add a
  file named like a secret (`.env`, `.env.*`, `*.jks`, `*.p12`, `*.pem`,
  `*.key`, `client_secret*` and the like). Files already tracked are
  warnings at `guard check` until `[guard.paths]` sets `ratchet`, and files
  under test and fixture folders are always warnings, so upgrading turns
  nothing red. Keep a path with `[guard.paths] allow`, or turn the rule off
  with `guard.paths = false` ([Secret paths](guard.md)). A secret already
  committed stays in history: rotate it.
- **Run `fairlead ci workflow --write` again** if Fairlead generated your
  workflow, to add the `ledger` job that answers "is this red mine?" when a
  job fails ([Plans in CI](ci.md)).
- **`fairlead done` plans from the merge base** with `[pr] base`, else the
  default branch, and says so in one line when it finds neither.
- **A `[pr]` table** is new and off until you add it ([Pull requests](pr.md)).
  With one, a body that says "verified" or "tested" needs a passing
  receipt for the head's tree; `[pr] claims = false` turns that off.

### To 0.19.0

`fairlead migrate --write` moves the floor. One thing changes what the
gate does, with no config change:

- **`fairlead done` runs under a budget**, `done.budget_seconds`, 300 by
  default. It runs the tests nearest the change until the budget is spent
  and records the rest of what the change reaches as owed to CI, so no
  suite holds the gate until `done --full` and the Stop hook stops sending
  an agent back for a suite its machine can't finish. The pass line and
  `fairlead receipt` say what is owed ([The budget](done.md#the-budget)).
  `done.budget_seconds = 0` keeps the gate as it was.
- **Give a slow suite a smoke set.** When only a `plan.run_all` path or a
  wide `[[graph.edges]]` rule reaches a suite, the gate runs the runner's
  `smoke` files and owes the rest; with none it runs nothing of that
  suite, and says so as advice.
- **The first budgeted run of a runner runs its nearest 20 files**, since
  nothing has timed it yet; the next run fits the budget from that time.
- **Debts are recorded and printed, not yet enforced.** The CI that runs
  the plan on the pull request pays them. A check that fails a pull
  request whose debts never ran is planned for the next release, so keep
  each need's `proved_in` until then.
- **`fairlead insights --score`** is new: the share of gate runs whose
  plan selected everything, how long the gate takes, and what was owed
  ([Score](insights.md#score)).

### To 0.19.1

Migrate updates nothing in your files. `config check` and `doctor` now
warn about a runner whose command names no `{files}`: add it where the
test files go, or the runner runs its whole suite on every change that
reaches one of its tests. `fairlead init` gives Playwright specs their own
runner, and sets up the guard alone in a repository with no tests yet.

## Going back to an older release

Install the older release, then run its `hooks uninstall` and
`hooks install`, so the hooks are the ones it writes. A config whose floor
names a newer release than the binary fails to load with the release it
needs; lower the floor only if the config uses nothing newer.
