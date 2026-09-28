# Changelog

## Unreleased

### New

- Vue, Svelte and Astro components are in the import graph: each `<script>` block, and Astro's frontmatter, is parsed as its `lang` says, a `<script src>` counts as an import, and a change to a component reaches the tests that import it through other components. On the vitest benchmark, recall is unchanged and the share of plans that ran everything fell from 34.9% to 34.7%.

## Unreleased

### New

- Replay recognises a failure wave after a runner image changes: `replay fetch` records each failed job's image and version, and failures of one test in one job across three or more unrelated pull requests, within 7 days of a new image version and never before it, get the outcome `environment`, listed by wave and kept out of adjusted recall. See [Runner image waves](https://mahkassem.github.io/fairlead/docs/replay.html#runner-image-waves).
- External graph providers: a `[[graph.providers]]` entry names a command that prints `{"version": 1, "edges": [...]}` for the files it claims, so any language or build tool can feed the plan. The built-in JavaScript and TypeScript scanner runs as the `typescript` provider, unchanged. A provider that fails runs every test for a change to its files, with a `provider-failed` warning, and `graph stats` reports each provider's files and edges. The output schema is committed as `provider-v1.schema.json`. See [Other languages](https://mahkassem.github.io/fairlead/docs/graph.html#other-languages-external-providers).

## 0.5.0 (2026-09-28)

### New

- Replay recognises failures a pull request inherited from its base branch: when the base's own push run failed the same test in the same job, or three or more unrelated pull requests on one base did. They get their own outcome, `inherited`, listed by group and kept out of adjusted recall, hits and misses alike. See [Inherited failures](https://mahkassem.github.io/fairlead/docs/replay.html#inherited-failures).
- `fairlead replay run` prints progress on stderr: every 25 planned runs and at least once a minute, with the failures judged, the misses so far and an estimate of the time left, and a line for any run slower than 10 seconds. `--quiet` turns it off; the report doesn't change.
- `fairlead replay fetch --event push` also records the default branch's push runs, and `replay run` plans each against its first parent, the diff the merged pull request's plan saw. A failing test that plan left out is reported as an escape, on its own `push (after merge)` line, so a repository whose pull requests run only the plan can still measure what got past it. See [`replay fetch`](https://mahkassem.github.io/fairlead/docs/replay.html#replay-fetch).

### Fixed

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
