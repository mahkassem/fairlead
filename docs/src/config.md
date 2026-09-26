# Configuration

Fairlead reads one file at the root of your repository: `fairlead.toml`, or `fairlead.yaml` if you prefer YAML. Both formats mean exactly the same thing. Every key has a default, so a repository without a config file works too.

```bash
fairlead config check          # validate every layer and the merged result
fairlead config show           # the merged config
fairlead config show --origin  # each value, with the layer that set it
fairlead config schema         # JSON Schema, for editor autocomplete
```

## Layers

Later layers override earlier ones:

1. Built-in defaults.
2. The project file: `fairlead.toml` or `fairlead.yaml`, found by walking up from the current directory, never past the repository root (the first directory holding `.git`). Two in the same directory is an error.
3. The local file, `fairlead.local.toml` (or `.yaml`), next to the project file. Keep it out of git. It's ignored when the `CI` environment variable is set.
4. Environment variables: `FAIRLEAD_` followed by the key path, with sections separated by a double underscore. So `FAIRLEAD_TESTS__UNREACHED=all` sets `tests.unreached`.
5. `--set key=value`, for one run: `fairlead config show --set tests.unreached=all`.

For environment variables and `--set`, a value that parses as a TOML boolean, number or array (`true`, `30`, `["a", "b"]`) is used as that type; anything else, dates included, is a string. Environment variables apply in name order.

## Lists

Lists append across layers, so your `plan.run_all` adds to the built-in one. To replace a list instead, write it as `{ replace = [...] }`:

```toml
[tests]
match = { replace = ["test/**/*.ts"] }
```

## Errors

Unknown keys are errors, so a typo can't silently switch something off. Every error names the file or layer it came from, and the key:

```text
fairlead.toml: tests.unreachd: unknown field `unreachd`, expected one of ...
```

## Keys

| Key | Default | Meaning |
| --- | --- | --- |
| `fairlead` | none | The oldest Fairlead version this config needs, such as `"0.2"` |
| `modules.discover` | `["workspaces"]` | Where modules come from |
| `modules.define` | `[]` | Extra modules: `{ pattern = "services/{name}/src" }` |
| `graph.tsconfig` | `"auto"` | The nearest tsconfig to each file, or a path |
| `graph.type_imports` | `true` | Count `import type` as an edge |
| `graph.unresolved` | `"warn"` | `warn` or `fail` on imports that don't resolve |
| `graph.conditions` | `["import", "node", "default"]` | Package `exports` conditions, in order |
| `graph.cache` | `true` | Keep parse results under `.git/fairlead` so unchanged files aren't parsed again |
| `tests.match` | `**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}` | Test files |
| `tests.exclude` | `**/node_modules/**` | Paths that are never test files |
| `tests.unreached` | `"module"` | What a changed file nothing reaches selects: `module`, `all` or `warn` |
| `tests.runners` | `[]` | `id`, `match`, `exclude` (files under `match` left to another runner), `invoke` (`once` or `per-module`), `cwd`, `command` |
| `tests.owners` | `[]` | Tests that don't import what they test: `match`, `covers` |
| `tests.classes` | `[]` | `class` (`unit`, `own`, `demand`, `canary`) for a `match` |
| `checks` | `[]` | Steps that aren't tests: `id`, `command`, `paths`, `modules`, `files` |
| `plan.run_all` | lockfiles, root manifests, tsconfig, runner and CI config | A changed path matching one selects everything |
| `plan.ignore` | root Markdown, changeset notes (`.changeset/*.md`), `docs/**`, READMEs, changelogs, licences | A changed path matching one selects nothing unless a file references it |
| `replay.provider` | `"github"` | Where CI history comes from |
| `replay.window_days` | `90` | How far back replay looks |
| `replay.min_failures` | `30` | Failures needed before a replay result counts |
| `replay.failures` | `[]` | `runner`, `extractor` (`vitest`, `jest`, `regex`), `job`, `pattern` |
| `replay.checks` | `[]` | Map a CI `job` and `step` to a `check` |
| `replay.ignore` | `[]` | CI job names (regexes) whose failures replay leaves out on purpose, such as a job that only aggregates others |
| `replay.ignore_steps` | `[]` | CI step names (regexes): a job that failed only in such steps, such as an install, is left out |

Commands are always argv arrays, never shell strings, and `{files}` expands to one argument per file. In `tests.runners.cwd`, `{module}` is the module's root path and `{module.id}` its id.

`config check` validates every key's type, the ids, placeholders and references between sections, and lists test files that match no runner or more than one. In `tests.owners`, a placeholder used in `covers` must be captured in `match`.
