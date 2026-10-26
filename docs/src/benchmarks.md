# Benchmarks

Replay results on public repositories, written by the `bench` workflow. See [Replay](replay.md) for what each measure means; the configs are in `bench/`.

Planned with Fairlead 0.13.0 at `081f41fc5e91`.

## Effect-TS/effect

Window 2026-06-08 to 2026-09-05; dataset `bench/data/Effect-TS_effect.jsonl`, 3075 run attempts from 2026-06-22 to 2026-09-05. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 558 |
| Attributed failures (gate 30) | 1520 (met) |
| Recall | 99.1% |
| Recall with quarantined tests counted (raw) | 97.5% (n=1661) |
| Recall with them left out (adjusted) | 99.1% (n=1520) |
| Strict recall (unconfirmed as misses) | 98.9% |
| Hits: selected, run everything, checks | 511, 705, 290 |
| Misses | 14 |
| Flaky, unconfirmed, unattributed | 68, 3, 67 |
| Unavailable, errors, ignored | 0, 0, 8 |
| Test files selected: median, p90 | 6.9%, 100.0% |
| Plans that selected everything | 18.9% |
| Plan time: first (cold), median, p90 | 1.70 s, 0.15 s, 0.53 s |
| Recall on `pull_request` runs (strict) | 99.1% (98.9%) |

Quarantined tests (declared in the bench config, applied only while the data bears them out):

- `packages/sql/mysql2/test/Persistence.test.ts` in jobs `^Test \(1/2, (Node Deno)\)$`: active, 21 failures absorbed (14 would-be hits, 7 would-be misses), 19 pull requests, until 2026-12-31. Times out after 30 s waiting on MySQL, on changes that touch neither: failed in 13 pull requests in July, only in the first test shard. Two of them broke the build instead (a transform error, a missing import).
  - Absorbed in: `Test (1/2, Deno)`, `Test (1/2, Node)`
- `packages/sql/mysql2/test/KeyValueStore.test.ts` in jobs `^Test \(2/2, (Node Deno)\)$`: active, 9 failures absorbed (8 would-be hits, 1 would-be misses), 9 pull requests, until 2026-12-31. Its setup hook times out after 30 s waiting on MySQL: failed in 8 pull requests in July, only in the second test shard. Two of them broke the build instead (a transform error, a missing import).
  - Absorbed in: `Test (2/2, Deno)`, `Test (2/2, Node)`
- `packages/sql/d1/test/Resolver.test.ts` in jobs `^Test \(2/2, Node\)$`: unverified, 0 failures absorbed (0 would-be hits, 0 would-be misses), 9 pull requests, until 2026-12-31. Times out after 5 s against the local D1 engine: failed in 7 pull requests in July, only in the second Node test shard.
  - Also failed in: `Test (Bun)`

Plans that selected everything, by cause:

- `unreached packages/**`: 23
- `run-all tsconfig.base.json`: 18
- `run-all tsconfig.packages.json`: 18
- `run-all package.json`: 16
- `run-all .github/**`: 10
- `run-all packages/**`: 5
- `unreached .agents/**`: 4
- `run-all pnpm-workspace.yaml`: 2
- `run-all vitest.config.ts`: 2
- `unreached .vscode/**`: 2

Failed jobs no rule watches:

- `AI Documentation Generation`: 7
- `Circular Dependencies`: 8
- `Test on Bun`: 8

Misses:

- run 29435902211 attempt 1: `packages/sql/pg/test/Client.test.ts`, changed `.changeset/cli-wizard-mode.md, packages/effect/src/unstable/cli/Command.ts, packages/effect/src/unstable/cli/GlobalFlag.ts, packages/effect/src/unstable/cli/Param.ts, packages/effect/src/unstable/cli/internal/ansi.ts`
- run 29544337445 attempt 2: `packages/effect/test/schema/toArbitrary.test.ts`, changed `.changeset/fix-cluster-mssql-for-update.md, packages/effect/src/unstable/cluster/SqlMessageStorage.ts, packages/platform-node/package.json, packages/platform-node/test/cluster/SqlMessageStorage.test.ts, packages/platform-node/test/cluster/SqlMessageStorageMssql.test.ts`
- run 30312917536 attempt 1: `packages/sql/libsql/test/Client.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30414439050 attempt 1: `packages/tools/openapi-generator/test/OpenApiGeneratorCli.test.ts`, changed `.changeset/add-deno-multipart.md, packages/platform-deno/src/DenoMultipart.ts, packages/platform-deno/src/index.ts`
- run 30414439050 attempt 1: `packages/tools/openapi-generator/test/JsonSchemaGeneratorRepresentation.test.ts`, changed `.changeset/add-deno-multipart.md, packages/platform-deno/src/DenoMultipart.ts, packages/platform-deno/src/index.ts`
- run 30690677178 attempt 1: `packages/sql/pg/test/Persistence.integration.test.ts`, changed `.changeset/mcp-protocol-versions.md, packages/effect/MCP.md, packages/effect/src/unstable/ai/McpProtocol.ts, packages/effect/src/unstable/ai/McpSchema.ts, packages/effect/src/unstable/ai/McpServer.ts`
- run 30711795071 attempt 1: `packages/sql/pg/test/Client.integration.test.ts`, changed `packages/effect/test/SubscriptionRef.test.ts`
- run 30899876531 attempt 1: `packages/platform-node/test/NodeRedis.integration.test.ts`, changed `packages/effect/test/unstable/cli/Help.test.ts`
- run 30899876531 attempt 1: `packages/sql/libsql/test/Client.integration.test.ts`, changed `packages/effect/test/unstable/cli/Help.test.ts`
- run 30955200961 attempt 1: `packages/platform-deno/test/cluster/SocketRunner.test.ts`, changed `.changeset/good-cups-reply.md, packages/sql/mssql/src/MssqlClient.ts, packages/sql/mssql/test/Client.test.ts`
- run 31047937195 attempt 1: `packages/platform-node/test/cluster/SqlMessageStorage.integration.test.ts`, changed `packages/ai/openai/test/OpenAiSchema.test.ts`
- run 31654685741 attempt 1: `packages/sql/mssql/test/Persistence.integration.test.ts`, changed `.changeset/scope-message-storage-clear-address.md, packages/effect/src/unstable/cluster/MessageStorage.ts, packages/effect/src/unstable/cluster/SqlMessageStorage.ts, packages/effect/test/cluster/MessageStorage.test.ts, packages/platform/node/test/cluster/MessageStorageTest.ts`
- run 33872621422 attempt 1: `packages/sql/d1/test/Resolver.test.ts`, changed `.changeset/docgen-class-property-examples.md, packages/tools/docgen/src/Core.ts, packages/tools/docgen/test/Core.propertyExamples.fixtures.ts, packages/tools/docgen/test/Core.propertyExamples.test.ts`
- run 33872655573 attempt 1: `packages/tools/openapi-generator/test/OpenApiGenerator.test.ts`, changed `packages/tools/api-diff/src/MigrationDoc.ts, packages/tools/api-diff/test/MigrationDoc.fence.test.ts`

## pnpm/pnpm

Window 2026-06-01 to 2026-08-29; dataset `bench/data/pnpm_pnpm.jsonl`, 3700 run attempts from 2026-06-25 to 2026-08-29. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 511 |
| Attributed failures (gate 30) | 665 (met) |
| Recall | 99.8% |
| Strict recall (unconfirmed as misses) | 99.8% |
| Hits: selected, run everything, checks | 96, 568, 0 |
| Misses | 1 |
| Flaky, unconfirmed, unattributed | 15, 0, 257 |
| Unavailable, errors, ignored | 0, 0, 555 |
| Test files selected: median, p90 | 100.0%, 100.0% |
| Plans that selected everything | 83.9% |
| Plan time: first (cold), median, p90 | 1.33 s, 0.47 s, 0.58 s |
| Recall on `pull_request` runs (strict) | 99.8% (99.8%) |

Plans that selected everything, by cause:

- `unreached pnpm/**`: 161
- `run-all pnpm-lock.yaml`: 120
- `unreached Cargo.lock`: 31
- `run-all .github/**`: 29
- `unreached pacquet/**`: 19
- `unreached .gitignore`: 14
- `unreached cspell.json`: 12
- `run-all package.json`: 9
- `run-all pnpm-workspace.yaml`: 4
- `unreached .typos.toml`: 1

Misses:

- run 31980676618 attempt 1: `pnpm11/installing/commands/test/fetch.ts`, changed `.changeset/typed-test-option-helpers.md, pnpm11/installing/deps-installer/test/lockfile.ts, pnpm11/installing/deps-installer/test/utils/testDefaults.ts, pnpm11/installing/deps-restorer/test/index.ts, pnpm11/installing/deps-restorer/test/utils/testDefaults.ts`

## vitest-dev/vitest

Window 2026-07-29 to 2026-10-26; dataset `bench/data/vitest-dev_vitest.jsonl`, 2253 run attempts from 2026-06-21 to 2026-10-26. Fetch: `fetch complete: 134 new rows in out/data.jsonl`.

| Measure | Value |
| --- | --- |
| Runs replayed | 1032 |
| Attributed failures (gate 30) | 765 (met) |
| Recall | 99.2% |
| Recall with quarantined tests counted (raw) | 95.6% (n=1325) |
| Recall with them left out (adjusted) | 99.2% (n=765) |
| Strict recall (unconfirmed as misses) | 99.0% |
| Hits: selected, run everything, checks | 570, 100, 89 |
| Misses | 6 |
| Flaky, unconfirmed, unattributed | 35, 2, 1515 |
| Unavailable, errors, ignored | 0, 0, 1576 |
| Test files selected: median, p90 | 98.5%, 100.0% |
| Plans that selected everything | 25.5% |
| Plan time: first (cold), median, p90 | 0.69 s, 0.18 s, 0.54 s |
| Recall on `pull_request` runs (strict) | 99.2% (99.0%) |

Quarantined tests (declared in the bench config, applied only while the data bears them out):

- `test/typescript/test/typechecker.test.ts` in jobs `^Test: unit, node-24, windows-latest$`: active, 88 failures absorbed (72 would-be hits, 15 would-be misses), 44 pull requests, until 2026-12-31. Fails only on the Windows unit job (its out-of-memory crash and missing-command cases): 60 failures across 36 pull requests from June to August 2026, while the same job passed 356 times and no other job ever failed it.
  - Absorbed in: `Test: unit, node-24, windows-latest`

Plans that selected everything, by cause:

- `run-all .github/**`: 118
- `run-all package.json`: 47
- `run-all pnpm-lock.yaml`: 30
- `run-all pnpm-workspace.yaml`: 19
- `unreached .gitattributes`: 12
- `run-all test/**`: 8
- `unreached .dockerignore`: 8
- `unreached eslint.config.js`: 4
- `unreached knip.jsonc`: 2
- `unreached test/**`: 2

Failed jobs no rule watches:

- `Browser: chromium, macos-latest`: 1
- `Browser: chromium, windows-latest`: 1
- `lint`: 1
- `test (macos-14, 20)`: 1
- `test (ubuntu-latest, 18)`: 1
- `test (ubuntu-latest, 20)`: 1
- `test (ubuntu-latest, 22)`: 1
- `test (windows-latest, 20)`: 1
- `test-browser (chrome, chromium)`: 1
- `test-browser (edge, webkit)`: 1
- `test-browser (firefox, firefox)`: 1
- `test-browser-windows (chrome, chromium)`: 1
- `test-browser-windows (edge, webkit)`: 1
- `test-ui`: 1
- `test-ui-e2e (macos-14)`: 1
- `test-ui-e2e (ubuntu-latest)`: 1
- `test-ui-e2e (windows-latest)`: 1

Misses:

- run 34299387859 attempt 1: `test/e2e/test/open-telemetry.test.ts`, changed `packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/pages/index.vue, test/ui/test/trace.spec.ts`
- run 34458638078 attempt 1: `test/coverage-test/test/reporters.test.ts`, changed `packages/ui/client/components/BrowserIframe.vue, packages/ui/client/styles/main.css`
- run 34956251133 attempt 1: `test/browser/specs/oxc.test.ts`, changed `packages/ui/client/pages/index.vue, packages/ui/client/styles/main.css`
- run 35604155400 attempt 1: `test/unit/test/expect-poll.test.ts`, changed `test/unit/test/jest-expect.test.ts`
- run 37770032024 attempt 1: `test/browser/specs/server-options-inline.test.ts`, changed `test/coverage-test/test/uncovered-concurrent-transform.istanbul.test.ts`
- run 39071518814 attempt 1: `test/e2e/test/mocking.test.ts`, changed `packages/coverage-v8/src/index.ts, test/coverage-test/test/extended-run-context.v8.test.ts`
