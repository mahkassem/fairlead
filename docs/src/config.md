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

1. Built-in defaults, then any [framework packs](graph.md#framework-packs) the files below name in `extends`.
2. *Since 0.10.0:* the workspace file, when the repository is in a [workspace](workspace.md) folder. Its `[workspace]` table is left out, and it's ignored when the `CI` environment variable is set.
3. The project file: `fairlead.toml` or `fairlead.yaml`, found by walking up from the current directory, never past the repository root (the first directory holding `.git`). Two in the same directory is an error.
4. The environment's shared file, `fairlead.<name>.toml` (or `.yaml`), when an environment is named. See [Environments](#environments).
5. The local file, `fairlead.local.toml` (or `.yaml`), next to the project file. Keep it out of git. It's ignored when the `CI` environment variable is set.
6. The environment's local file, `fairlead.<name>.local.toml`, also out of git and ignored in CI.
7. Environment variables: `FAIRLEAD_` followed by the key path, with sections separated by a double underscore. So `FAIRLEAD_TESTS__UNREACHED=all` sets `tests.unreached`.
8. `--set key=value`, for one run: `fairlead config show --set tests.unreached=all`.

For environment variables and `--set`, a value that parses as a TOML boolean, number or array (`true`, `30`, `["a", "b"]`) is used as that type; anything else, dates included, is a string. Environment variables apply in name order.

## Environments

Name an environment with `FAIRLEAD_ENV=staging`, or `--env staging` on any command, and Fairlead layers `fairlead.staging.toml` over the project file and `fairlead.staging.local.toml` over the local one. A file with no `.local` is shared and belongs in git; one with `.local` is yours and stays out of it, like `fairlead.local.toml`.

```text
fairlead.toml                 every environment
fairlead.staging.toml         staging, shared
fairlead.local.toml           you, every environment
fairlead.staging.local.toml   you, in staging
```

Environment files sit beside the project file, wherever you run from. An environment name is lowercase letters, digits and dashes, and can't be `local`. Naming an environment with no file for it is an error, so a typo can't fall back to the defaults silently. In CI only the shared file counts, so an environment CI uses needs one. `replay run --config` reads exactly the file it names, with no environment or local layers.

## Lists

Lists append across layers, so your `plan.run_all` adds to the built-in one. To replace a list instead, write it as `{ replace = [...] }`:

```toml
[tests]
match = { replace = ["test/**/*.ts"] }
```

So `run_all = []` appends nothing and the built-in list stays; `config check` warns about an empty list written over one that already has items. `{ replace = [] }` clears it.

## Globs

`*` matches within a path segment and `**` across segments. `{a,b}` is an alternation, and alternations nest: `{src/**/*.test.{ts,tsx},tests/**}`. A brace group of just a name, like `{name}`, captures that segment instead, where the key supports captures. `[abc]` is a character class. A backslash makes the next character literal, so a path with brackets is `app/**/\[...slug\]/**` (in TOML's double-quoted strings the backslash itself is doubled: `"app/**/\\[...slug\\]/**"`). `config check` compiles every glob, so one that can't compile fails there, not at plan time.

## Errors

Unknown keys are errors, so a typo can't silently switch something off. Every error names the file or layer it came from, and the key:

```text
fairlead.toml: tests.unreachd: unknown field `unreachd`, expected one of ...
```

## Keys

| Key | Default | Meaning |
| --- | --- | --- |
| `fairlead` | none | The oldest Fairlead version this config needs, such as `"0.2"`. *Since 0.7.0:* a config that an older binary can't read, because it uses keys added later, gets this version in the error instead of the unknown key. |
| `modules.discover` | `["workspaces"]` | Where modules come from |
| `modules.define` | `[]` | Extra modules: `{ pattern = "services/{name}/src" }` |
| `graph.tsconfig` | `"auto"` | The nearest tsconfig to each file, or a path |
| `graph.type_imports` | `true` | Count `import type` as an edge |
| `graph.unresolved` | `"warn"` | `warn` or `fail` on imports that don't resolve |
| `graph.conditions` | `["import", "node", "default"]` | Package `exports` conditions, in order |
| `graph.cache` | `true` | Keep parse results under `.git/fairlead` so unchanged files aren't parsed again |
| `extends` | `[]` | *Unreleased:* framework packs layered under this config: a built-in pack's name, such as `"laravel"`, or a pack file's path. See [Framework packs](graph.md#framework-packs) |
| `graph.edges` | `[]` | `{ from, to }` rules: each file matching `from` depends on the files `to` matches. See [Import graph](graph.md#edges-the-imports-dont-show). *Unreleased:* `find`, a regex searched in each `from` file's text, whose capture fills `{1}` in `to`. See [Names in the code](graph.md#names-in-the-code) |
| `graph.barrier` | `[]` | Globs the walk reaches but doesn't go past. See [Import graph](graph.md#barriers) |
| `graph.providers` | `[]` | `{ id, command, files, timeout_seconds }`: external commands that print the graph for the files they claim. See [Import graph](graph.md#other-languages-external-providers) |
| `tests.match` | `**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}` | Test files |
| `tests.exclude` | `**/node_modules/**` | Paths that are never test files |
| `tests.unreached` | `"module"` | What a changed file nothing reaches selects: `module`, `all` or `warn` |
| `tests.runners` | `[]` | `id`, `match`, `exclude` (files under `match` left to another runner), `invoke` (`once` or `per-module`), `cwd`, `command`. *Since 0.7.1:* `all_command` (the argv when everything runs) and `exclude_arg` (an argv fragment that leaves one file out), below. *Since 0.8.0:* `from`, the earliest [CI stage](ci.md#stages) it runs at, `ready` by default |
| `tests.owners` | `[]` | Tests that don't import what they test: `match`, `covers`, and `overrides_run_all` to let a covered `plan.run_all` path select only those tests |
| `tests.classes` | `[]` | `class` (`unit`, `own`, `demand`, `canary`) for a `match` |
| `checks` | `[]` | Steps that aren't tests: `id`, `command`, `paths`, `modules`, `files`. *Since 0.8.0:* `from`, the earliest [CI stage](ci.md#stages) it runs at, `draft` by default |
| `quarantine` | `[]` | A test (`path`) or check (`check`) that fails on one platform whatever the change: `os`, `when` (`autocrlf`, `space-in-path`), `signature`, `reason`, `proved_in`, `until` ([Test plan](plan.md#tests-that-lie-on-one-platform)) |
| `plan.run_all` | lockfiles, root manifests, tsconfig, runner and CI config | A changed path matching one selects everything. *Since 0.8.0:* except a change that only moves dependency versions and a workflow run only by hand or on a schedule ([Changes that don't run everything](plan.md#changes-that-dont-run-everything)) |
| `plan.lockfile` | `"all"` | `all`: a changed lockfile selects everything; `scope` (opt-in): a changed root `pnpm-lock.yaml` selects the workspace packages whose resolved dependencies changed |
| `plan.ignore` | root Markdown, the changesets tool's folder (`.changeset/**`), `docs/**`, READMEs, changelogs, licences | A changed path matching one that reaches no test selects nothing, instead of falling to `tests.unreached`; one that reaches tests still selects them, and a changed test file always runs |
| `replay.provider` | `"github"` | Where CI history comes from |
| `replay.window_days` | `90` | How far back replay looks |
| `replay.min_failures` | `30` | Failures needed before a replay result counts |
| `replay.failures` | `[]` | `runner`, `extractor` (`vitest`, `jest`, `bun`, `phpunit`, `pest`, `go`, `pytest`, `maven`, `gradle`, `regex`), `job`, `pattern` |
| `replay.checks` | `[]` | Map a CI `job` and `step` to a `check` |
| `replay.ignore` | `[]` | CI job names (regexes) whose failures replay leaves out on purpose, such as a job that only aggregates others |
| `replay.quarantine` | `[]` | Tests declared flaky in named jobs: `path`, `job`, `reason`, `until` ([Replay](replay.md#quarantine)) |
| `replay.ignore_steps` | `[]` | CI step names (regexes): a job that failed only in such steps, such as an install, is left out |
| `guard.baseline` | `"fairlead-baseline.json"` | Ratcheted counts by file and rule, relative to the project root |
| `guard.exclude` | `[]` | Tracked files no guard rule reads |
| `guard.findings` | `"added"` | Which findings count at a commit: `added`, only those the change adds; `all`, every finding in a file it touches |
| `guard.on_finding` | `"deny"` | `deny` stops the commit on those findings; `warn` shows them and lets it through |
| `guard.budget_ms` | `40` | The write hook's own time; past it the edit goes ahead and the event log says `timed_out` |
| `guard.events` | `"local"` | `local` records decisions in `.git/fairlead/events.jsonl`; `off` records nothing |
| `guard.cite` | `{}` | A note appended to a rule's messages, by rule id |
| `guard.size` | off | `files`, `exclude`, `file_lines`, `function_lines`, `test_hooks`, and `ratchet` (default `true`). See [Guard rules](guard.md) |
| `guard.test_names` | off | `files`, `exclude`, `file`, `titles_without`, `title_calls`. See [Guard rules](guard.md#test-names) |
| `guard.citations` | off | `files`, `exclude`, `pattern` (with a `code` group), `headings_in`. See [Guard rules](guard.md#citations) |
| `guard.migrations` | off | `files`, `immutable` (default `true`), `base`, `unique_prefix`. See [Guard rules](guard.md#migrations) |
| `guard.commands` | `[]` | `match`, `reason`: commands an agent may not run; `unless`, a regex that lets a matched command through (*Since 0.8.0*). See [Guard rules](guard.md#commands) |
| `guard.external` | `[]` | `id`, `command`, `stages` (default `["check"]`, or `commit`), `ratchet`. See [Guard rules](guard.md#external-rules) |
| `hooks.claude` | `"shared"` | Where `fairlead hooks install` puts the Claude Code hooks: `shared`, `.claude/settings.json`; `local`, `.claude/settings.local.json` |
| `done.tests` | `"planned"` | What `fairlead done` runs of the plan's test invocations: `planned` or `none`. See [The done gate](done.md) |
| `done.checks` | `"planned"` | The same for the `[[checks]]` the plan selects |
| `done.always` | `[]` | `[[checks]]` ids `fairlead done` runs for every change |
| `done.guard` | `true` | Whether `fairlead done` ends with `fairlead guard check` |
| `done.on_stop` | `"ask"` | What the Claude Code Stop hook does while the tree hasn't passed: `off`, `ask` (send the agent back once) or `require`. See [The done gate](done.md#the-stop-hook) |
| `brief.per` | `"session"` | Whether a second `fairlead brief` in the same session adds its paths to the first (`session`) or stands alone (`call`). See [The brief](brief.md) |
| `brief.nudge` | `true` | Whether `fairlead hooks install` adds the `PostToolUse` note after an edit made with no brief |
| `brief.resume` | `true` | Whether `fairlead hooks install` adds the `SessionStart` hook that runs `fairlead resume`. See [Context and resume](context.md#the-sessionstart-hook). *Since 0.8.0* |
| `memory.dir` | `".fairlead/lessons"` | Where the [lesson](lessons.md) files live. *Since 0.8.0* |
| `memory.review_days` | `90` | Days from `added` to the `review_by` a new lesson gets |
| `memory.max_lines` | `12` | The longest body a lesson may have |
| `memory.cap` | `5` | Lessons a brief lists before "N more" |
| `memory.learn` | `"write"` | What `fairlead learn` does: `write` the file to the working tree, or `ask`, which prints it for a person to save |
| `skills.imports` | `1` | Hops along what a changed file imports, for [skills](skills.md) and lessons alike. *Since 0.8.0* |
| `skills.importers` | `0` | Hops along the files that import a changed file; off by default |
| `skills.cap` | `8` | Skills a brief lists before "N more" |
| `skills.targets` | `["claude", "agents", "cursor"]` | The agents `skills sync` writes for |
| `skills.routes` | `[]` | `{ skill, paths, modules, always }`: the SKILL.md and the code it applies to. See [Skills](skills.md) |
| `agents.write` | `"block"` | What `fairlead agents sync` does: `block` keeps the marked block in each file; `never` writes nothing and prints it for a person to copy in. See [AGENTS.md and CLAUDE.md](agents.md). *Since 0.8.0* |
| `agents.files` | `["AGENTS.md", "CLAUDE.md"]` | The files that carry the block, relative paths inside the repository |
| `stages.environments` | `["main"]` | Branches, by name or glob, whose pushes are the merge stage. See [Stages](ci.md#stages). *Since 0.8.0* |
| `stages.full_label` | `"run-everything"` | A pull request label that makes its runs the full stage |
| `stages.reuse` | `true` | At merge, skip the ready stage's steps when the pull request passed them on the same tree |
| `ci.comment` | `false` | Whether `fairlead ci report` also keeps one pull request comment up to date with the report. See [Plans in CI](ci.md#fairlead-ci-report---plan-path) |
| `ci.escapes` | `"report"` | What a failing test the merged change's plan left out does once `ci run --judge` finds it: `"report"` lists it, `"fail"` also makes `ci report` exit 1. See [Escapes](ci.md#escapes---judge-path) |
| `guard.comments` | off | `files`, `exclude`, `tests`, `migrations`, `block_length`, `density`, `history`, `item_codes`, `agent_phrases`, `block_marker`, and `ratchet` (default `false`). See [Guard rules](guard.md#comments) |

Commands are always argv arrays, never shell strings, and `{files}` expands to one argument per file. `{packages}` expands to one `./dir` per directory holding a selected test, or `./...` when everything runs, for `go test`. *Since 0.7.0:* for Maven and Gradle, an argument holding `{class}` is repeated once per selected test class and `{classes}` joins them with commas; either is dropped when everything runs ([Java and Kotlin](graph.md#java-and-kotlin)). In `tests.runners.cwd`, `{module}` is the module's root path and `{module.id}` its id.

*Since 0.7.1:* when everything runs, `{files}` expands to nothing, so `bun test {files}` becomes a bare `bun test`, which runs every test the tool finds, end-to-end specs included, not only the files the runner's `match` claims. A runner's `all_command` is the argv to use instead whenever everything runs, such as `["bun", "test", "test/unit"]`; without one, `command` runs with no files, as before. It takes the same placeholders as `command` except `{files}`, `{class}` and `{classes}`, which have nothing to expand to there and fail `config check`: `{packages}` is `./...`, and `{module}` and `{module.id}` are filled in per module. `exclude_arg` is an argv fragment holding `{file}`, such as `["--exclude", "{file}"]` or `["--ignore={file}"]`, added once for each test a [`[[quarantine]]` entry](plan.md#tests-that-lie-on-one-platform) holds, so a run of everything can leave out the tests that run alone ([Invocations](plan.md#invocations)).

`config check` validates every key's type, the ids, placeholders and references between sections, and lists test files that match no runner or more than one. In `tests.owners`, a placeholder used in `covers` must be captured in `match`.
