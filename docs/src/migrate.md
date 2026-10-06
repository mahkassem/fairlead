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
| Claude Code hooks in `.claude/settings.json` and `settings.local.json` | Fairlead's entries aren't the ones this release's `hooks install` would write for your config | Fairlead's entries are replaced with this release's, and every other setting and hook stays as it was. Uninstall still restores the file from before Fairlead, byte for byte, when nobody else changed it. |
| The commit stage in `lefthook.yml` | It runs another command than this release writes, such as `fairlead` by name where the project has its own copy | The `run:` line is replaced in place, with its indentation, quotes and comments kept |
| The `fairlead = "X.Y"` floor in the project config | The floor is older than a table or key the config uses, such as `[done]` (0.6) or `[guard]` (0.4) | The floor is raised to that release, so a teammate on an older binary is told which version to install |
| `package.json` | `dependencies`, `devDependencies` or `optionalDependencies` pins an older release, as `0.4.2`, `^0.4.2`, `~0.4.2` or `>=0.4.2` | The version moves to this release and keeps its `^` or `~` |
| Workflows under `.github/workflows/` | A step uses `mahkassem/fairlead@vX.Y.Z`, or passes `version: vX.Y.Z`, for an older release | Both move to this release |

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

## Going back to an older release

Install the older release, then run its `hooks uninstall` and
`hooks install`, so the hooks are the ones it writes. A config whose floor
names a newer release than the binary fails to load with the release it
needs; lower the floor only if the config uses nothing newer.
