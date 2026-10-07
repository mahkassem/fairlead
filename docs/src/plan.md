# Test plan

`fairlead plan` works out which tests and checks the changes since a base commit can affect, and says why each one is in. It gates on recall: a plan must include every test the change can break. Selecting fewer tests is the goal, never the rule.

```sh
fairlead plan                      # changes since the remote's default branch
fairlead plan --base main --json   # the plan as JSON (schema below)
fairlead plan --files src/a.ts     # plan for these files or directories, no git needed
fairlead test --explain test/a.test.ts   # why a test or check is in, or isn't
```

The working tree is always the head: in CI that's the checked-out commit. The base is the merge base of `--base` (default: `origin/HEAD`, then `origin/main`, `origin/master`, `main`, `master`) and `HEAD`. If there's no merge base, as in a shallow clone, the plan fails with exit code 2 rather than planning nothing; fetch more history (`fetch-depth: 0`) or pass `--files`.

## How a plan is built

1. **Changed paths.** `git diff --name-status --find-renames` from the base to the working tree, plus untracked files. A rename counts both paths; its old path counts as deleted.
2. **Run everything.** A changed path matching `plan.run_all` selects every test and check. The defaults are lockfiles, root manifests, tsconfig files, test-runner and task-runner config, and CI workflows. With `plan.lockfile = "scope"` (opt-in for now), a root `pnpm-lock.yaml` is the exception when it can be read: see [Lockfile changes](#lockfile-changes). *Since 0.8.0:* a change that only moves dependency versions, an edit to Fairlead's own config and a workflow that runs only by hand or on a schedule are exceptions too: see [Changes that don't run everything](#changes-that-dont-run-everything).
3. **Ignored paths.** A changed path matching `plan.ignore` still selects every test it reaches through the files that depend on it. When it reaches none, it's listed under `ignored` and selects nothing, where it would otherwise fall to `tests.unreached`. That holds for source files too, imported or not: a unit test's fake, or a dev tool's code, reaches no end-to-end spec. A changed test file is never ignored. So a layer that plans a different kind of test (say `--env e2e`) can list the unit tests, their fakes and dev tools here. The defaults are root Markdown, the changesets tool's folder (`.changeset/**`), `docs/**`, READMEs, changelogs and licences.
4. **Deleted files.** A deleted file goes back into the graph as a phantom, joined to every import that now fails but would resolve to it, every path literal that names it, and its package, so whatever depended on it still counts.
5. **Package manifests.** A changed `<package>/package.json` counts as every file in that package changing.
6. **The walk.** From every changed file to everything that depends on it, over [import graph](graph.md) edges. A file with a non-literal dynamic import, a local import that doesn't resolve, or a tsconfig that couldn't be applied depends on every file in its module; at the root, on every file.
7. **Tests** (below), **unreached files**, then **checks** and the **invocations** that run them.


## Lockfile changes

With `plan.lockfile = "scope"`, a changed root `pnpm-lock.yaml` (lockfile versions 6 to 9) is compared with the base's. Each workspace package whose own entry changed, or that depends, in the base or the head lockfile, on a package whose entry changed, counts as if its `package.json` changed: every file in it starts the walk, and checks and owner rules watching that manifest see it. A package nested inside an affected one is affected too, since Node resolution walks up.

It still selects everything when:

- `plan.lockfile = "all"`, the default until the [benchmarks](benchmarks.md) have more evidence;
- there's no base (as with `--files`);
- pnpm hoists packages into a shared `node_modules` (`node-linker=hoisted`, `shamefully-hoist` or a `public-hoist-pattern` in `.npmrc` or `pnpm-workspace.yaml`);
- the root package's dependencies changed, since every package sees them;
- an affected entry isn't a workspace package in the head tree;
- anything outside `importers`, `packages`, `snapshots` and `catalogs` changed, such as `overrides`, `patchedDependencies` or `settings`;
- or the lockfile doesn't parse, has no importers (a single-project lockfile), or names a package it doesn't list.

A lockfile change that reaches no package selects nothing and says so in a `lockfile-scoped-to-nothing` warning.

Other lockfiles (`package-lock.json`, `yarn.lock`, `bun.lockb`) always select everything, and so does a `bun.lock` change that's more than versions moving (below).

## Changes that don't run everything

*Since 0.8.0:* three kinds of change match `plan.run_all` or reach no test, yet can't change what a test does. Each says what it did in a warning, so `fairlead plan` and `--explain` aren't silent about it.

**Dependency versions moving** (`version-bump-scoped`). A root or workspace `package.json`, or the root `bun.lock`, whose change only moves versions selects the files that import the moved packages, and the tests that reach those, instead of everything. Only moving versions means the parsed JSON differs only in the version strings of `dependencies`, `devDependencies`, `peerDependencies` and `optionalDependencies` entries, and in the package's own `version`; for `bun.lock`, only in package entries' resolved versions, integrity and dependency versions, and in each workspace's dependency versions. A package that depends on a moved one, by name in the base or the head `bun.lock`, counts as moved too, since it loads the new version. The import graph keeps every bare import from outside the repository, whether it resolved into `node_modules` or the package isn't installed, so an import names the package even without an install. A test's chain starts at `package.json (left-pad)`. A package nothing imports, such as a command-line devDependency, selects nothing, and a package's own `version` alone selects nothing either.

It still selects what it did before (everything for the root files, the whole package for a workspace manifest) when:

- the change does more than move versions: a script, `workspaces`, `overrides`, a package added or removed, a version that becomes a `workspace:`, `file:`, `npm:` or git specifier, and anything that doesn't parse;
- there's no base text (as with `--files`), the file was added, deleted or renamed, or there's no `bun.lock` at the head (npm, yarn and pnpm projects);
- a moved package, or one depending on it, is how a `[[tests.runners]]` command runs, by its name or a command it installs (`version-bump-runs-everything`);
- a file matching `plan.run_all`, such as `vitest.config.ts`, imports one (`version-bump-runs-everything`);
- or one is named in text where no import shows it: a file matching `plan.run_all` (other than a `package.json` or a lockfile) or the `scripts` of the root or a changed `package.json` holds its name, or a command it installs, as a whole word, bounded by quotes, `/`, whitespace, a shell or list separator or either end. That catches a `preset: 'ts-jest'`, a plugin list, `"types": ["vitest/globals"]` and a CLI a script runs; the warning names the file. A devDependency named only in the manifest and the lockfile still selects nothing.

**Fairlead's own config** (`fairlead-config`). `fairlead.toml`, `fairlead.yaml` or a layer such as `fairlead.ci.toml`, at the root, selects nothing by itself: the plan is made with the new config. It used to fall to `tests.unreached`, since no test imports it. A config file you list in `plan.run_all` still selects everything.

**A workflow run only by hand or on a schedule** (`workflow-dispatch-only`). A `.github/workflows/*.yml` or `*.yaml` file whose only triggers are `workflow_dispatch` and `schedule` can't change a pull request's or a push's run, so it selects nothing by itself. The `on:` key is read at the head and at the base (a deleted file only at the base, an added one only at the head). If either side has any other trigger, `workflow_call` and `workflow_run` included, or doesn't parse, or the base text is missing, it selects everything as before.

Checks and owner rules whose `paths` match one of these files still see it change.

## Modules

A module is a unit a plan can widen to: each workspace package (`modules.discover = ["workspaces"]`), and each directory a `modules.define` pattern matches, named by its `{name}`. A file belongs to the deepest module above it; a file in none is at the root.

## Tests and classes

Test files are those matching `tests.match` and not `tests.exclude`. When runners are configured, each test must match exactly one `[[tests.runners]]`; `fairlead config check` lists any that match none or two, and a plan refuses to run with them. A test's class is set by the first `[[tests.classes]]` rule that matches, `unit` by default.

| Class | Selected when |
| --- | --- |
| `unit` | it changed, it depends on a changed file, or an owner rule claims it |
| `own` | it changed, or an owner rule claims it |
| `demand` | it changed (never under `run_all`) |
| `canary` | every plan |

An owner rule selects tests that don't import what they test. A changed path matching one of its `covers` selects the tests its `match` names, with any `{name}` the change captured filled in, so a change under `services/api/src/` picks `services/api/test/` and not every service's tests:

```toml
[[tests.owners]]
match = "services/{name}/test/integration/**"
covers = ["services/{name}/src/**"]
```

A path an owner rule covers still selects everything when it matches `plan.run_all`, unless the rule sets `overrides_run_all = true`. Then it selects that rule's tests, plus whatever depends on it. That fits a fixture project's runner config, which `**/vitest.config.*` matches but only one suite's tests load:

```toml
[[tests.owners]]
match = "test/{suite}/**"
covers = ["test/{suite}/**/fixtures/**", "test/{suite}/vitest.config.*"]
overrides_run_all = true
```

Don't set it for a file something outside the rule's tests reads, such as a config another package's config imports or a script names with `--config`. When the rule claims no test for the path, it runs everything as before.

## Unreached files

A changed file that isn't a test and has no test anywhere among the files depending on it is *unreached* (a deleted test file and a package manifest aren't, since there's nothing left to run for one and the other already stands for its package): something the graph can't see uses it, like a setup file named only in a runner config. It's always listed under `unreached` with a warning, so you can write the owner rule that covers it. Unless one already claims a test for it, `tests.unreached` decides what it selects. An owner rule whose `match` names no test file in this config, as can happen when a layer replaces `tests.match`, doesn't count as covering the path, and `config check` warns about it:

| `tests.unreached` | Selects |
| --- | --- |
| `module` (default) | its module's tests; everything when it's at the root or its module has no tests |
| `all` | everything |
| `warn` | nothing |

## Checks

A `[[checks]]` entry is selected when its `paths` match a changed file (deleted files included, since removing a file can break a typecheck), when its `modules` include one the change reaches, or in every plan when it names neither. `{files}` in its command expands to the changed files its `paths` match, deleted ones left out; with `files = "matched"`, or under `run_all`, to every file they match.

## Invocations

The plan lists what to run as `invocations`, each an argv and a working directory. `{files}` expands to one argument per file.

- A runner with `invoke = "once"` gets one invocation with every selected test.
- A runner with `invoke = "per-module"` gets one per module holding selected tests, in `cwd` with `{module}` (the module's root) and `{module.id}` (its name) filled in, with files relative to that directory.
- Under `run_all` or a widening to everything, `{files}` expands to nothing, so each runner runs its whole suite, per module for per-module runners. *Since 0.7.1:* a runner with `all_command` runs that instead of `command`, for a tool that, given no files, finds more tests than the runner's `match` claims ([Configuration](config.md)).
- Each selected check follows the runners.
- A test or check a `[[quarantine]]` entry holds on this machine runs alone, after the rest (below).

## Tests that lie on one platform

*Since 0.7.0:* some tests fail on one machine whatever the change. A test that splits a file on `\n` fails on a Windows checkout with `core.autocrlf=true`, since every line keeps its `\r`. A test that runs a process in `new URL(..., import.meta.url).pathname` fails with `ENOENT` on Windows, where that path is `/C:/...`, and wherever the checkout's path has a space, which it spells `%20`. Each one looks like a regression until someone diagnoses it by hand, and the next agent on that machine starts cold and pays the same again. A `[[quarantine]]` entry records the diagnosis once:

```toml
[[quarantine]]
path = "test/headings.test.ts"       # a test file, or `check = "<id>"` for a [[checks]] entry
when = ["autocrlf"]                  # conditions detected here, all of which must hold
signature = "has no heading"         # a regex over the failure's output
reason = "it splits docs/decisions.md on LF only, so a CRLF checkout keeps a CR on every heading"
proved_in = "CI on Linux"
until = "2026-12-31"

[[quarantine]]
path = "test/tracked-files.test.ts"
os = "windows"                       # windows, macos or linux
signature = "spawnSync git ENOENT"
reason = "it runs git in a file URL's pathname, which is /C:/... on Windows"
proved_in = "CI on Linux"
until = "2026-12-31"
```

An entry holds where its `os` matches and every condition in `when` is detected, and only until its `until` date. The conditions are detected, never declared: `autocrlf` when git's `core.autocrlf` is true for the repository, and `space-in-path` when the repository's path has a space in it. An entry needs an `os`, a `when` or both, so it can't hold everywhere. Elsewhere it does nothing, so the same plan on Linux runs the test as usual.

Where an entry holds:

- `fairlead plan` lists the test or check under *not provable here*, with what made the entry hold, its evidence, where it is proved instead and its date. The plan JSON lists it in `quarantined`, and the invocation that runs it carries its name in `quarantined`.
- It still runs, alone, so its failure can be read by itself. Its runner's other tests run together without it, and are named one by one even in a plan that runs everything. *Since 0.7.1:* in a plan that runs everything, a runner with `exclude_arg` keeps its whole run (its `all_command`, or `command` with no files) and adds the fragment once per held test file, relative to the invocation's working directory, so the tool still finds the rest. A runner without it names its other tests, which drops any the tool would find that `match` doesn't claim and can make a long command line, so the plan warns `quarantine-narrowed-everything`. A runner whose command has no `{files}` can't run one test alone, so the plan warns `quarantine-not-separable` and the test runs with the rest.
- `fairlead done` and `fairlead ci run` don't count a failure whose output matches `signature`. They report it as not provable here, so the agent can say so in the pull request instead of diagnosing it again or skipping it quietly. A failure with any other output is real, and counts. An entry excuses one failure: when the held file's output names more than one failed test, read the way replay reads that runner (Vitest, Jest, bun, pytest, PHPUnit, Pest or `go test`), the run counts. A runner whose output Fairlead can't read is judged by `signature` alone.
- The machine that runs a plan checks the entry again: a plan made where an entry holds excuses nothing on a machine where its OS or conditions don't hold.
- After `until`, the entry stops holding. The plan warns `quarantine-expired`, and a failure counts again.

`fairlead test --explain` says the same for one test, or why an entry doesn't hold here. `fairlead config check` names the conditions it detects on the machine and whether each entry holds, and `fairlead doctor` names the conditions.

`[[replay.quarantine]]` is a different tool: it keeps a test that flakes in named CI jobs out of replay's recall ([Replay](replay.md#quarantine)).

## Reasons

Every test and check carries the reason that put it in first: `run-all`, `changed`, `import` with the chain from the change to the test, `owner`, `canary`, `unreached`, and for checks `paths`, `modules` or `always`. `fairlead test --explain` prints the chain for a selected test, and for one that isn't selected, why not. *Since 0.9.0:* each hop that isn't a plain import names its kind, such as `(type import)`, `(path literal)` or `(coverage)`, and a hop the walk adds conservatively says why it's there:

```
$ fairlead test --explain test/pricing.test.ts
test/pricing.test.ts is selected (unit):
src/a.ts
  -> src/api.ts  (unresolved import `./gen/missing`, so it depends on its whole module)
  -> src/types.ts  (type import)
  -> test/pricing.test.ts  (type import)
```

The other conservative hops read `a dynamic import with no literal path` or `its tsconfig couldn't be applied`, and a workspace import that resolved to no file names the specifier and the package it's kept as a dependency on. `graph why` names each edge's kind the same way.

## Plan JSON, version 1

`fairlead plan --json` prints the plan; `--out PATH` also writes it. Write it outside the working tree, or ignore it in git, or the next plan counts it as a change. The shape is a public contract with a version field: new fields are additive, so a reader must accept fields it doesn't know, and optional fields are left out when empty; anything else bumps the version. The schema is committed as [`plan-v1.schema.json`](plan-v1.schema.json) and printed by `fairlead plan --schema`.

- `plan_id`: stable for the same Fairlead version, config, tree, base and changes.
- `config_digest`: `sha256:` of the merged config.
- `tree_hash`: `HEAD`'s git tree id when the working tree is clean, else `worktree:` and a hash of every file's path and blob id.
- `head`: `HEAD`'s commit when the working tree is clean, else `worktree`.
- `base`, `all`, `changed`, `ignored`, `tests`, `checks`, `invocations`, `unreached` and `warnings` as described above.
- `quarantined`: the selected tests and checks a `[[quarantine]]` entry holds on the machine that made the plan, left out when there are none. Each has its `target` (the test file or check id), `kind`, what made it hold (`here`), `signature`, `reason`, `proved_in` and `until`.
