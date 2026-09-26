# Benchmarks

Replay results on public repositories, written by the `bench` workflow. See [Replay](replay.md) for what each measure means; the configs are in `bench/`.

Planned with Fairlead 0.1.1 at `346943dbb521`.

## Effect-TS/effect

Window 2026-05-02 to 2026-07-30; dataset `bench/data/Effect-TS_effect.jsonl`, 700 run attempts from 2026-06-22 to 2026-07-30. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 102 |
| Attributed failures (gate 30) | 823 (met) |
| Recall | 98.4% |
| Strict recall (unconfirmed as misses) | 98.4% |
| Hits: selected, run everything, checks | 39, 713, 58 |
| Misses | 13 |
| Flaky, unconfirmed, unattributed | 24, 0, 10 |
| Unavailable, errors, ignored | 0, 0, 1 |
| Test files selected: median, p90 | 98.9%, 100.0% |
| Plans that selected everything | 38.2% |
| Plan time: first (cold), median, p90 | 1.58 s, 0.20 s, 1.43 s |
| Recall on `pull_request` runs (strict) | 98.4% (98.4%) |

Plans that selected everything, by cause:

- `unreached packages/**`: 11
- `run-all package.json`: 10
- `run-all .github/**`: 8
- `run-all packages/**`: 4
- `run-all vitest.config.ts`: 2
- `unreached scripts/**`: 2
- `run-all pnpm-lock.yaml`: 1
- `run-all tsconfig.base.json`: 1

Failed jobs no rule watches:

- `AI Documentation Generation`: 1
- `Circular Dependencies`: 1

Misses:

- run 29435902211 attempt 1: `packages/sql/pg/test/Client.test.ts`, changed `.changeset/cli-wizard-mode.md, packages/effect/src/unstable/cli/Command.ts, packages/effect/src/unstable/cli/GlobalFlag.ts, packages/effect/src/unstable/cli/Param.ts, packages/effect/src/unstable/cli/internal/ansi.ts`
- run 29544337445 attempt 2: `packages/effect/test/schema/toArbitrary.test.ts`, changed `.changeset/fix-cluster-mssql-for-update.md, packages/effect/src/unstable/cluster/SqlMessageStorage.ts, packages/platform-node/package.json, packages/platform-node/test/cluster/SqlMessageStorage.test.ts, packages/platform-node/test/cluster/SqlMessageStorageMssql.test.ts`
- run 30312917536 attempt 1: `packages/platform-node/test/NodeRedis.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30312917536 attempt 1: `packages/sql/mysql2/test/Persistence.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30312917536 attempt 1: `packages/platform-node/test/cluster/SqlRunnerStorage.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30312917536 attempt 1: `packages/sql/libsql/test/Client.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30324294308 attempt 1: `packages/sql/mysql2/test/Persistence.test.ts`, changed `.changeset/fresh-lines-wait.md, packages/platform-node-shared/src/NodeTerminal.ts, packages/platform-node-shared/test/NodeTerminal.test.ts, packages/platform-node-shared/test/fixtures/node-terminal.ts`
- run 30324294308 attempt 1: `packages/sql/libsql/test/Client.test.ts`, changed `.changeset/fresh-lines-wait.md, packages/platform-node-shared/src/NodeTerminal.ts, packages/platform-node-shared/test/NodeTerminal.test.ts, packages/platform-node-shared/test/fixtures/node-terminal.ts`
- run 30330015500 attempt 1: `packages/sql/mysql2/test/Persistence.test.ts`, changed `.changeset/eff-140-deno-crypto.md, packages/platform-deno/src/DenoCrypto.ts, packages/platform-deno/src/index.ts, packages/platform-deno/test/DenoCrypto.test.ts`
- run 30390581785 attempt 2: `packages/sql/libsql/test/Resolver.test.ts`, changed `.changeset/fix-reactive-query-metadata.md, packages/effect/src/unstable/reactivity/AtomHttpApi.ts, packages/effect/src/unstable/reactivity/AtomRpc.ts, packages/effect/test/reactivity/AtomHttpApi.test.ts, packages/effect/test/reactivity/AtomRpc.test.ts`
- run 30401970132 attempt 1: `packages/sql/mysql2/test/KeyValueStore.test.ts`, changed `.changeset/eff-145-deno-services.md, packages/platform-deno/src/DenoServices.ts, packages/platform-deno/src/index.ts`
- run 30414439050 attempt 1: `packages/tools/openapi-generator/test/OpenApiGeneratorCli.test.ts`, changed `.changeset/add-deno-multipart.md, packages/platform-deno/src/DenoMultipart.ts, packages/platform-deno/src/index.ts`
- run 30414439050 attempt 1: `packages/tools/openapi-generator/test/JsonSchemaGeneratorRepresentation.test.ts`, changed `.changeset/add-deno-multipart.md, packages/platform-deno/src/DenoMultipart.ts, packages/platform-deno/src/index.ts`

## pnpm/pnpm

Window 2026-04-18 to 2026-07-16; dataset `bench/data/pnpm_pnpm.jsonl`, 1000 run attempts from 2026-06-25 to 2026-07-16. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 179 |
| Attributed failures (gate 30) | 234 (met) |
| Recall | 99.1% |
| Strict recall (unconfirmed as misses) | 99.1% |
| Hits: selected, run everything, checks | 40, 192, 0 |
| Misses | 2 |
| Flaky, unconfirmed, unattributed | 2, 0, 126 |
| Unavailable, errors, ignored | 0, 0, 188 |
| Test files selected: median, p90 | 100.0%, 100.0% |
| Plans that selected everything | 82.0% |
| Plan time: first (cold), median, p90 | 1.21 s, 0.38 s, 0.52 s |
| Recall on `pull_request` runs (strict) | 99.1% (99.1%) |

Plans that selected everything, by cause:

- `unreached pnpm/**`: 38
- `run-all pnpm-lock.yaml`: 33
- `run-all .github/**`: 19
- `unreached pacquet/**`: 15
- `unreached .gitignore`: 14
- `unreached Cargo.lock`: 12
- `run-all package.json`: 6
- `unreached cspell.json`: 3
- `unreached .typos.toml`: 1

Misses:

- run 29268600635 attempt 1: `pnpm11/releasing/commands/test/change/index.test.ts`, changed `pnpm11/installing/deps-installer/test/install/verifyLockfileResolutionsCache.ts`
- run 29268600635 attempt 1: `pnpm11/releasing/commands/test/change/index.test.ts`, changed `pnpm11/installing/deps-installer/test/install/verifyLockfileResolutionsCache.ts`

## vitest-dev/vitest

Window 2026-05-30 to 2026-08-27; dataset `bench/data/vitest-dev_vitest.jsonl`, 977 run attempts from 2026-06-21 to 2026-08-27. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 806 |
| Attributed failures (gate 30) | 195 (met) |
| Recall | 92.8% |
| Strict recall (unconfirmed as misses) | 92.3% |
| Hits: selected, run everything, checks | 86, 62, 33 |
| Misses | 14 |
| Flaky, unconfirmed, unattributed | 1, 1, 3088 |
| Unavailable, errors, ignored | 0, 0, 717 |
| Test files selected: median, p90 | 99.4%, 100.0% |
| Plans that selected everything | 45.5% |
| Plan time: first (cold), median, p90 | 1.70 s, 0.18 s, 0.34 s |
| Recall on `pull_request` runs (strict) | 92.8% (92.3%) |

Plans that selected everything, by cause:

- `run-all .github/**`: 122
- `run-all test/**`: 86
- `run-all pnpm-lock.yaml`: 69
- `run-all package.json`: 43
- `unreached docs/**`: 26
- `unreached .dockerignore`: 8
- `unreached eslint.config.js`: 3
- `run-all examples/**`: 2
- `unreached knip.jsonc`: 2
- `run-all pnpm-workspace.yaml`: 1

Misses:

- run 28767749426 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `test/browser/specs/console.test.ts, test/browser/specs/in-source.test.ts, test/browser/specs/isolate-setup.test.ts, test/browser/specs/runner.test.ts, test/browser/specs/stacktrace.test.ts`
- run 28834064643 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `test/e2e/test/__snapshots__/stacktraces.test.ts.snap`
- run 30582865157 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `packages/browser/src/node/index.ts, test/e2e/test/config/browser-configs.test.ts`
- run 30828513835 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `test/browser/test/commands.test.ts`
- run 31092084755 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `packages/browser/src/client/tester/runner.ts`
- run 31169965084 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `test/e2e/test/reporters/github-actions.test.ts`
- run 31178965161 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `packages/browser/src/node/index.ts`
- run 31351752762 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `examples/projects/package.json, pnpm-lock.yaml`
- run 31357939266 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `docs/api/vi.md, docs/guide/mocking/classes.md, test/unit/test/mocking/vi-fn.test.ts`
- run 31358877832 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `docs/api/vi.md, docs/guide/mocking/classes.md, test/unit/test/mocking/vi-fn.test.ts`
- run 31359615129 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `docs/api/vi.md, docs/guide/mocking/classes.md, test/unit/test/mocking/vi-fn.test.ts`
- run 31360200429 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `docs/api/vi.md, docs/guide/mocking/classes.md, test/unit/test/mocking/vi-fn.test.ts`
- run 31382477355 attempt 1: `test/typescript/test/typechecker.test.ts`, changed `packages/browser/src/node/index.ts, test/e2e/test/config/browser-configs.test.ts`
- run 33076437839 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/coverage-v8/src/provider.ts, test/coverage-test/test/non-file-urls.unit.test.ts`
