# Changelog

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
