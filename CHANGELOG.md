# Changelog

## Unreleased

### New

- `fairlead ci run --results PATH` records each invocation's outcome and time, and `fairlead ci report` turns the plan, the results and an optional receipt into one Markdown summary: what was selected and why, what ran, and the command to run each failure again. It goes to the step summary, and with `--comment` or `ci.comment = true` to one pull request comment kept up to date. See [Plans in CI](https://mahkassem.github.io/fairlead/docs/ci.html#fairlead-ci-report---plan-path).
- `fairlead receipt` compares what changed with the session's brief: files it named, files in its reach, and files outside it with the tests each adds, then the done gate's state for the tree as it is. `fairlead next` prints the one step the change is waiting for: a brief, the gate, the failing step's command, the receipt, or nothing. See [The receipt and next](https://mahkassem.github.io/fairlead/docs/receipt.html).
- `fairlead brief <paths>` answers before an edit what the paths reach in the import graph, the tests and checks the plan will select, the `[guard.*]` rules that read them and the done gate's steps, each with its source, in at most 40 lines. A brief is kept per agent session, and a second one adds its paths (`brief.per`). `fairlead hooks install` also adds a `PostToolUse` hook that tells an agent once per session, after its first edit, when it has no brief (`brief.nudge`). See [The brief](https://mahkassem.github.io/fairlead/docs/brief.html).
- A Claude Code `Stop` hook, `fairlead guard stop`, installed with the others: while the tree an agent leaves hasn't passed `fairlead done`, it sends the agent back with the reason, once under `done.on_stop = "ask"` (the default) or every time under `"require"`. See [The Stop hook](https://mahkassem.github.io/fairlead/docs/done.html#the-stop-hook).
- `fairlead done` runs the gate a change passes before it counts as finished (the planned tests and checks, the checks `done.always` names, and the guard) and records the outcome against the tree it checked, so any later edit makes a pass stale. `--check` says whether the working tree as it stands has passed. See [The done gate](https://mahkassem.github.io/fairlead/docs/done.html).

## 0.5.1 (2026-09-29)

### Fixed

- A changed path matching `plan.ignore` is ignored when it reaches no test, even if other files import it, such as a fake that only unit tests use or a dev tool's modules. In 0.5.0 an imported path was never ignored and fell to `tests.unreached`, so in a layer planning end-to-end specs it could run all of them. An ignored path that reaches tests still selects them.

## 0.5.0 (2026-09-29)

### New

- Coverage maps: `fairlead coverage import --format phpunit-xml|coverage-py` turns a coverage run's report into a map of which files each test ran, stamped with its commit and date, and `[graph.coverage] map = ...` adds those edges to the static graph. A dependency only a run shows, such as a class a container builds from a string or a module imported by name, then reaches its tests. An old map leaves a `coverage-stale` warning and an unreadable one `coverage-unreadable`; neither fails the plan. The map's shape is committed as `coverage-v1.schema.json`. See [Coverage maps](https://mahkassem.github.io/fairlead/docs/graph.html#coverage-maps).
- Replay reads `go test` failures, plain or `-v`, and gotestsum's, with the `go` extractor. Each failed top-level test is attributed to the test file it logged, else a test file in its panic, else a compile error in a test file, under the package's import path, and attribution matches such a path by its tail, even for a module in a subfolder.
- Python files are in the import graph, as the built-in `python` provider: each import reaches its module and the package `__init__.py` files above it, `from a.b import c` reaches `a.b.c` when it's a module and `a.b` otherwise, relative imports resolve from the file's package, and a test or `conftest.py` depends on every `conftest.py` above it. Modules resolve under the root, each Python project's folder and its `src`. Replay reads pytest failures with the `pytest` extractor. Python manifests and lockfiles are in the default `plan.run_all`.
- Go files are in the import graph, as the built-in `go` provider: an import depends on every non-test file of the package, a test file on every other file of its directory, and a `//go:embed` pattern on the files it names. Import paths resolve through every `go.mod` in the tree with their local `replace` directives, then the importing module's `vendor/`. On opentelemetry-go, 29 modules and 1,280 files, the graph builds in 0.05 s, and for four changed packages the test packages it selects are exactly the ones `go list -test -deps` says depend on them. A runner's `{packages}` expands to `./dir` per selected package, or `./...` when everything runs, for `go test`. `go.mod`, `go.sum`, `go.work`, `go.work.sum`, `composer.json` and `composer.lock` are in the default `plan.run_all`.
- Replay reads PHPUnit and Pest failures, with the `phpunit` and `pest` extractors; `pest` also reads Laravel's `php artisan test`. The test file comes from the stack frame or `at` line whose file name is the test class's, else from the class name, so an error thrown in application code isn't mistaken for the test file, and a runner's absolute path is matched by its longest tail in the repository. `replay fetch` keeps those lines in the dataset.
- PHP files are in the import graph, as the built-in `php` provider: class references, `use` imports, called functions and literal `include`/`require` paths, resolved through the namespace and imports, then composer's `psr-4` and `psr-0` prefixes from every `composer.json` in the tree, then the files that declare each name. On laravel/framework's 2,986 PHP files the graph builds in about a second, a tenth of that from the parse cache. PHP tests aren't matched by default. See [PHP](https://mahkassem.github.io/fairlead/docs/graph.html#php).
- Vue, Svelte and Astro components are in the import graph: each `<script>` block, and Astro's frontmatter, is parsed as its `lang` says, a `<script src>` counts as an import, and a change to a component reaches the tests that import it through other components. On the vitest benchmark, recall is unchanged and the share of plans that ran everything fell from 34.9% to 34.7%.
- Replay recognises a failure wave after a runner image changes: `replay fetch` records each failed job's image and version, and failures of one test in one job across three or more unrelated pull requests, within 7 days of a new image version and never before it, get the outcome `environment`, listed by wave and kept out of adjusted recall. See [Runner image waves](https://mahkassem.github.io/fairlead/docs/replay.html#runner-image-waves).
- External graph providers: a `[[graph.providers]]` entry names a command that prints `{"version": 1, "edges": [...]}` for the files it claims, so any language or build tool can feed the plan. The built-in JavaScript and TypeScript scanner runs as the `typescript` provider, unchanged. A provider that fails runs every test for a change to its files, with a `provider-failed` warning, and `graph stats` reports each provider's files and edges. The output schema is committed as `provider-v1.schema.json`. See [Other languages](https://mahkassem.github.io/fairlead/docs/graph.html#other-languages-external-providers).
- A Go module's `go.mod` and `go.sum` are graph dependencies of its files (edge kind `manifest`) instead of `plan.run_all` defaults, so a dependency bump in one module of a multi-module repository selects that module and its importers, not everything. Under a `go.work`, which picks versions across its modules, a file depends on every one of their manifests. `go.work` itself still runs everything; add `**/go.mod` to `plan.run_all` to keep the old behaviour.
- Replay recognises failures a pull request inherited from its base branch: when the base's own push run failed the same test in the same job, or three or more unrelated pull requests on one base did. They get their own outcome, `inherited`, listed by group and kept out of adjusted recall, hits and misses alike. See [Inherited failures](https://mahkassem.github.io/fairlead/docs/replay.html#inherited-failures).
- `fairlead replay run` prints progress on stderr: every 25 planned runs and at least once a minute, with the failures judged, the misses so far and an estimate of the time left, and a line for any run slower than 10 seconds. `--quiet` turns it off; the report doesn't change.
- `fairlead replay fetch --event push` also records the default branch's push runs, and `replay run` plans each against its first parent, the diff the merged pull request's plan saw. A failing test that plan left out is reported as an escape, on its own `push (after merge)` line, so a repository whose pull requests run only the plan can still measure what got past it. See [`replay fetch`](https://mahkassem.github.io/fairlead/docs/replay.html#replay-fetch).

### Fixed

- The `pest` extractor also reads PHPUnit's failure format, which Laravel's `php artisan test --parallel` prints; on a Laravel application's CI those failures used to be unattributed.
- Replay attributes a failing test file a Windows runner printed with backslashes, such as pytest's `FAILED tests\unit\test_a.py::test_x`; it used to match no file and count as unattributed.
- Replay's `go` extractor reads the file testify names on a line of its own (`x_test.go:12:`), so a testify failure is attributed to its test file instead of its package.
- `replay fetch` retries a 5xx from GitHub up to three times, after 5, 10 and 15 seconds; one transient 500 used to stop the whole fetch.
- Globs take nested alternations (`{src/**/*.test.{ts,tsx},tests/**}`) and backslash escapes (`\[...slug\]`); both used to fail at plan time. `config check` now compiles every glob the planner compiles (test matches, runners, owners, `plan.run_all`, `plan.ignore`), so a bad one fails there instead of the first time a change reaches it.
- `config check` warns when a layer writes `[]` over a list that already has items: lists append, so it changes nothing, and `{ replace = [] }` is what clears one.
- Replay no longer bloats a blobless clone: checking whether a recorded commit is present made git lazily fetch it without negotiation, resending the whole history's trees each time, and a bench clone grew to 13 GiB. Commit lookups now set `GIT_NO_LAZY_FETCH` (git 2.44 or later), and missing commits come through the one negotiated fetch.
- `plan.ignore` applies to source files too: a changed source nothing imports, matching it, selects nothing instead of falling to `tests.unreached`. An imported source still reaches its tests and a changed test file always runs. This changes the defaults' behaviour for an unimported source under `docs/**`, which used to widen to every test ([#124](https://github.com/mahkassem/fairlead/issues/124)).
- An owner rule whose `match` names no test file no longer counts as covering a changed path, so the path falls to `tests.unreached` instead of silently selecting nothing, and `config check` warns about such a rule. A rule written only to make paths select nothing should become a `plan.ignore` entry ([#124](https://github.com/mahkassem/fairlead/issues/124)).
- A file over 256 KB, scanned for imports rather than parsed, no longer reads prose such as a doc comment's `from "My booking"` as an import. The unresolved "import" used to tie the file, and every test importing it, to changes at the repository root ([#125](https://github.com/mahkassem/fairlead/issues/125)).
- `fairlead hooks install` in a repository that lists Fairlead as a package dependency writes hooks that run the project's own copy: the Claude Code hook calls the binary the npm package unpacked, falling back to the lockfile's package runner (`bun x`, `pnpm exec`, `yarn` or `npx --no-install`), and the lefthook entry runs through that runner. Before, both called `fairlead` by name, which a dev-dependency install doesn't put on the PATH, so the write hook silently let every edit through. `hooks status`, `doctor` and `uninstall` recognise a commit-stage entry run through a package runner or by path.
- Replay's `bun` extractor counts a test file that failed to load (an import or syntax error, which bun reports as an unhandled error under the file's header with no `(fail)` line) as that file's failure, instead of finding nothing. Datasets fetched before this keep no such lines, so re-fetch to see them.
- A captured segment containing glob characters, such as `[slug]`, now selects only its own files in owner rules and rule edges; the captured value used to be read as glob syntax. `{foo-bar}` and other brace groups whose name isn't letters, digits and `_` are no longer taken for placeholders by config validation, matching how patterns read them.
- The write hook no longer starts `git` to ask whether a migration exists at HEAD each time: it reads HEAD's commit from the ref files and looks the path up in a list of that commit's migration paths kept in `.git/fairlead/head-paths/`, which `guard check` and the commit stage fill and the hook fills with one `git ls-tree` when HEAD has moved. On a slow machine the old way could run past `guard.budget_ms` and let an edit to an existing migration through.

## 0.4.2 (2026-09-28)

### Fixed

- Generated npm packages no longer include `npm-shrinkwrap.json`.

## 0.4.0 (2026-09-27)

### New

- `fairlead guard compare FILE` checks the guard against the linter it replaces: it reads the other linter's `file:line rule` lines and fails on any finding one side has and the other doesn't, with `--map` for rule names and `--rules` to narrow it. `fairlead guard bench --since REV` replays the files each commit changed through the write hook and reports p50, p95 and the slowest time, and what it decided; `--p95-under` makes it a check. CI now fails when the write hook's p95 on a generated history goes over 50 ms. See [Checking the guard against your linter](https://mahkassem.github.io/fairlead/docs/guard.html#checking-the-guard-against-your-linter).
- The git hook: `fairlead hooks install --git` adds the commit stage to `lefthook.yml` by inserting lines, keeping its comments and layout, and uninstall restores it byte for byte. Plain `install` adds it wherever a lefthook config already exists. `fairlead doctor` reports both hooks, whether lefthook will run the git one, whether `fairlead` is on the PATH, and the event log: write-hook decisions, denies by rule, p50 and p95 time, time-outs, errors and commit-stage runs. See [The git hook](https://mahkassem.github.io/fairlead/docs/guard.html#the-git-hook).
- The write stage: `fairlead hooks install` adds a Claude Code hook that runs `fairlead guard hook` before every edit, and denies one that adds a finding, with the findings as the reason the agent reads (or, with `guard.on_finding = "warn"`, lets it through with them as a note). It never answers "allow", lets a call through whenever it can't decide, and keeps to `guard.budget_ms` (40 by default). `[[guard.commands]]` deny shell commands. `hooks.claude` chooses `.claude/settings.json` (shared, the default) or `.claude/settings.local.json`; `fairlead hooks uninstall` restores the file byte for byte when nobody changed it since. See [The write hook](https://mahkassem.github.io/fairlead/docs/guard.html#the-write-hook).

- More guard rules: `[guard.test_names]` (file names and test titles), `[guard.citations]` (a pointer in a comment must name a heading in a Markdown file), `[guard.migrations]` (a migration that exists at the base may not change, move or go, and numbers are unique), `[[guard.commands]]` (commands an agent may not run, for the write stage), and `[[guard.external]]` (any tool that prints `file:line message` becomes a rule, at the check stage by default or at commit). `guard check --base REV` checks migrations against a base. See [Guard rules](https://mahkassem.github.io/fairlead/docs/guard.html#test-names).
- Guard presets: `[guard.comments]` checks comment block length by context, comment density, history (dates, names, phrases, measurements), item references outside a pointer form, comments that address the next editor, and block markers, reading comments after code from the syntax tree. `[guard.size]` adds `function_lines`, counting a function's own lines without the functions nested in it. `[guard.cite]` appends a note to a rule's messages. See [Guard rules](https://mahkassem.github.io/fairlead/docs/guard.html#comments).
- Environments: `FAIRLEAD_ENV=staging` or `--env staging` layers `fairlead.staging.toml` (shared) over the project file and `fairlead.staging.local.toml` (personal, ignored in CI) over the local one. Naming an environment with no file for it is an error. See [Configuration](https://mahkassem.github.io/fairlead/docs/config.html#environments).
- `fairlead guard check` checks the project's own rules over every tracked file, with a ratchet: `--write-baseline` records today's counts for ratcheted rules and the check fails only when one rises. `--staged` is the commit stage, failing only on findings the staged change adds, compared by what each finding is about rather than its line. The first rule is `file-length` (`[guard.size]`). `guard.findings = "all"` counts every finding in a touched file instead, and `guard.on_finding = "warn"` shows the findings without stopping the commit. Commit-stage runs are recorded in `.git/fairlead/events.jsonl`, with rule ids and timings and never file contents; `guard.events = "off"` turns that off. See [Guard rules](https://mahkassem.github.io/fairlead/docs/guard.html).

### Changed

- A plan's config digest leaves out `guard` and `hooks`, which never change a plan, so digests are the same as 0.3.0's for the same config.
- `[[guard.external]]` refuses `stages = ["write"]`: when the write hook runs, the file isn't written yet.

## 0.3.0 (2026-09-26)

### New

- Edges the imports don't show: `[[graph.edges]]` makes each file matching `from` depend on the files its `to` globs match, and the planner follows the edge like an import. It fits a test that reaches its code over HTTP instead of importing it. A `{name}` is read from the side where it's a whole path segment, so `{area}{,-*}.test.ts` still picks up `orders-refunds.test.ts`, and `config check` refuses a rule that would silently match the wrong files. See [Import graph](https://mahkassem.github.io/fairlead/docs/graph.html#edges-the-imports-dont-show).
- A walk barrier: `graph.barrier` names files the planner reaches but doesn't go past, such as a server module that every area imports and that imports every area. A changed barrier file is left to `tests.unreached` unless `plan.run_all` covers it. `graph why` and `test --explain` say where a barrier stopped the walk, and `graph stats` counts rule edges and barrier files. See [Barriers](https://mahkassem.github.io/fairlead/docs/graph.html#barriers).
- Replay reads `bun test` output: `extractor = "bun"` attributes each `(fail)` line to the file header above it, including bun's interleaved parallel output and several bun runs in one job, and never takes a failure from bun's closing summary. The dataset keeps those lines. See [Replay](https://mahkassem.github.io/fairlead/docs/replay.html).

### Known gaps

- A bun test file that fails to load prints no `(fail)` line, so replay can't attribute it yet ([#68](https://github.com/mahkassem/fairlead/issues/68)).
- A captured path segment that looks like glob syntax, such as `[slug]`, is re-read as a glob when filled into another pattern ([#69](https://github.com/mahkassem/fairlead/issues/69)).

## 0.2.0 (2026-09-26)

### New

- `fairlead plan` lists the tests and checks the changes since a base commit can reach, and why each one is in. It builds an import graph of JavaScript and TypeScript from source, with no install, following tsconfig paths and workspace packages, and caches parsed files by git blob. It adds owner rules, test classes (`unit`, `own`, `demand`, `canary`), checks, and a run-everything fallback for anything it can't read with certainty. The plan is JSON with a published schema. See [Test plan](https://mahkassem.github.io/fairlead/docs/plan.html).
- `fairlead test --explain` says why a test or check is in the plan, or why it isn't.
- `fairlead ci plan` plans the checked-out commit and, with `--format github`, sets step outputs. `fairlead ci run --plan` runs the plan's invocations. The GitHub Action installs Fairlead and runs any command. See [Plans in CI](https://mahkassem.github.io/fairlead/docs/ci.html).
- `fairlead graph stats`, `why` and `importers` inspect the import graph.
- `fairlead replay fetch` records a repository's pull request and merge queue runs from GitHub. `fairlead replay run` re-plans every recorded failure and reports recall: each failure is a hit, a miss, flaky, unconfirmed, or quarantined. `[[replay.quarantine]]` declares a test flaky in named jobs, with evidence and an expiry, and applies only while the data bears it out. See [Replay](https://mahkassem.github.io/fairlead/docs/replay.html).
- Opt-in lockfile scoping (`plan.lockfile = "scope"`): a pnpm lockfile change runs only the packages whose resolved dependencies changed.
- An owner rule can take a run-all path it covers (`overrides_run_all`), such as a fixture project's runner config.
- Weekly public benchmarks on Effect, pnpm and vitest, published on the [benchmarks page](https://mahkassem.github.io/fairlead/docs/benchmarks.html).

### Changed

- The minimum Rust version for building from source is 1.90.
- The site has a front page, and the documentation moved to [/docs/](https://mahkassem.github.io/fairlead/docs/). Old page addresses redirect.
- Fairlead has a brand: the logo and its colors are in `assets/brand`.

### Recall

Replayed from each project's CI history (run 5 of the weekly benchmarks, planned at `8c1b179d44cc`). Adjusted recall leaves out tests quarantined with evidence; raw counts them.

| Repository | Window | Adjusted recall | Raw recall | Misses |
| --- | --- | --- | --- | --- |
| Effect-TS/effect | 2026-05-09 to 2026-08-06 | 98.8% (n=853) | 98.0% (n=890) | 10 |
| pnpm/pnpm | 2026-04-25 to 2026-07-23 | 99.4% (n=312) | 99.4% (n=312) | 2 |
| vitest-dev/vitest | 2026-06-13 to 2026-09-10 | 95.6% (n=528) | 93.6% (n=627) | 23 |

Selection is recorded, not yet a gate: the median plan still selects most test files (Effect 98.6%, pnpm 100.0%, vitest 98.7%), and 31.7%, 84.8% and 34.9% of plans ran everything, mostly for CI config, lockfile and `package.json` changes. Narrowing that is the next milestone's work.

A full graph build without the parse cache takes 1.19 s on Effect, 1.02 s on pnpm and 0.41 s on vitest on 4 cores, under the 1.5 s budget, which CI checks on every pull request ([Import graph](https://mahkassem.github.io/fairlead/docs/graph.html#speed)). That's with the files already in the operating system's cache; a first read from a cold disk takes longer.

Quarantined, each until 2026-12-31 and applied only while the data bears it out:

- Effect `packages/sql/mysql2/test/Persistence.test.ts` in the first test shard: times out after 30 s waiting on MySQL on changes that touch neither; 21 failures across 19 pull requests.
- Effect `packages/sql/mysql2/test/KeyValueStore.test.ts` in the second test shard: its setup hook times out waiting on MySQL; 9 failures across 9 pull requests.
- Effect `packages/sql/d1/test/Resolver.test.ts` in the second Node shard: times out after 5 s against the local D1 engine; 7 failures across 7 pull requests.
- vitest `test/typescript/test/typechecker.test.ts` in the Windows unit job only: out-of-memory crashes and missing-command cases. It was declared on 60 failures across 36 pull requests, while the same job passed 356 times, and has absorbed 100 failures across 54 pull requests.

Every miss, with its cause:

- Effect, 10: database and service tests failing together on unrelated changes (`sql-libsql` Client 3, `platform-node` NodeRedis 1, SqlRunnerStorage 1, `sql-pg` Client 1 on a LISTEN notification timeout, `sql-libsql` Resolver 1 on a 5 s timeout); `toArbitrary.test.ts` 1, a property test that failed after 8 random cases; `openapi-generator` 2, a native module error (`Failed to recover TsconfigCache type from napi value`) under Deno on a change to `platform-deno`.
- pnpm, 2: `releasing/commands/test/change/index.test.ts` on Linux and Windows, a missing `CHANGELOG.md` fixture, on a pull request that only edited a test helper in another package.
- vitest, 23: headless browser specs 14 (`runner.test.ts` 9, fixtures failing inside it: a 249 ms locator click timeout in 5, a CDP events test in 3 (both recurring across unrelated pull requests) and a WebKit clipboard test on Windows in 1; `trace.test.ts`, `bail-out.test.ts`, `locators.test.ts`, `server-url.test.ts` and `to-match-screenshot.test.ts` 1 each, timing and Windows snapshot mismatches); `detect-async-leaks.test.ts` 6, the same leak assertion across unrelated pull requests and bases; `list.test.ts` and `open-telemetry.test.ts` 2, a browser that didn't close within 10 s; `coverage-test/reporters.test.ts` 1, a 15 s timeout on Windows.

The bar we set for this release was 100% recall on failures a change could have caused. By our reading of the logs, none of these misses is one, but that's a judgement, not a measurement: literal 100% recall wasn't reached on any repository, and the misses above are the whole list.

### Fixed

- Replay waits out GitHub's secondary rate limit instead of stopping, and asks once per lookup.

## 0.1.1 (2026-09-26)

### New

- `fairlead config check`, `config show [--origin]` and `config schema`. One config file, `fairlead.toml` or `fairlead.yaml`, with layers: built-in defaults, the project file, a local file, `FAIRLEAD_*` environment variables and `--set`. Lists append across layers, and `{ replace = [...] }` replaces one. Every error names its file and key. See the [configuration docs](https://mahkassem.github.io/fairlead/config.html).
- `fairlead doctor` says whether the config is valid.

### Changed

- The npm package is published through npm trusted publishing, with provenance, from an approval-gated release environment. There's no npm token.
- The minimum Rust version for building from source is 1.89.

### Security

- CI checks workflows with zizmor, keeps the generated release workflow unchanged, and runs CodeQL.
- A hashed leak check keeps private names out of files, issues, pull requests, comments and commit messages.

## 0.1.0 (2026-09-25)

- First release: `fairlead --version` and `fairlead doctor`, installers for Linux, macOS and Windows, and the npm package.
