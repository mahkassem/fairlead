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
2. **Run everything.** A changed path matching `plan.run_all` selects every test and check. The defaults are lockfiles, root manifests, tsconfig files, test-runner and task-runner config, and CI workflows.
3. **Ignored paths.** A changed path matching `plan.ignore` selects nothing by itself and is listed under `ignored`, unless a file references it, in which case it counts like any other change. A source file is never ignored, so a config file under `docs/` still counts. The defaults are root Markdown, changeset notes (`.changeset/*.md`), `docs/**`, READMEs, changelogs and licences.
4. **Deleted files.** A deleted file goes back into the graph as a phantom, joined to every import that now fails but would resolve to it, every path literal that names it, and its package, so whatever depended on it still counts.
5. **Package manifests.** A changed `<package>/package.json` counts as every file in that package changing.
6. **The walk.** From every changed file to everything that depends on it, over [import graph](graph.md) edges. A file with a non-literal dynamic import, a local import that doesn't resolve, or a tsconfig that couldn't be applied depends on every file in its module; at the root, on every file.
7. **Tests** (below), **unreached files**, then **checks** and the **invocations** that run them.

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

## Unreached files

A changed file that isn't a test and has no test anywhere among the files depending on it is *unreached* (a deleted test file and a package manifest aren't, since there's nothing left to run for one and the other already stands for its package): something the graph can't see uses it, like a setup file named only in a runner config. It's always listed under `unreached` with a warning, so you can write the owner rule that covers it. Unless one already does, `tests.unreached` decides what it selects:

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
- Under `run_all` or a widening to everything, `{files}` expands to nothing, so each runner runs its whole suite, per module for per-module runners.
- Each selected check follows the runners.

## Reasons

Every test and check carries the reason that put it in first: `run-all`, `changed`, `import` with the chain from the change to the test, `owner`, `canary`, `unreached`, and for checks `paths`, `modules` or `always`. `fairlead test --explain` prints the chain for a selected test, and for one that isn't selected, why not.

## Plan JSON, version 1

`fairlead plan --json` prints the plan; `--out PATH` also writes it. Write it outside the working tree, or ignore it in git, or the next plan counts it as a change. The shape is a public contract with a version field: new fields are additive, so a reader must accept fields it doesn't know, and optional fields are left out when empty; anything else bumps the version. The schema is committed as [`plan-v1.schema.json`](plan-v1.schema.json) and printed by `fairlead plan --schema`.

- `plan_id`: stable for the same Fairlead version, config, tree, base and changes.
- `config_digest`: `sha256:` of the merged config.
- `tree_hash`: `HEAD`'s git tree id when the working tree is clean, else `worktree:` and a hash of every file's path and blob id.
- `head`: `HEAD`'s commit when the working tree is clean, else `worktree`.
- `base`, `all`, `changed`, `ignored`, `tests`, `checks`, `invocations`, `unreached` and `warnings` as described above.
