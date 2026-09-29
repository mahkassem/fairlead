# Benchmarks

Replay results on public repositories, written by the `bench` workflow. See [Replay](replay.md) for what each measure means; the configs are in `bench/`.

Planned with Fairlead 0.5.1 at `7410697f2dbb`.

## Effect-TS/effect

Window 2026-05-26 to 2026-08-23; dataset `bench/data/Effect-TS_effect.jsonl`, 2175 run attempts from 2026-06-22 to 2026-08-23. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 460 |
| Attributed failures (gate 30) | 1478 (met) |
| Recall | 98.2% |
| Recall with quarantined tests counted (raw) | 97.7% (n=1515) |
| Recall with them left out (adjusted) | 98.2% (n=1478) |
| Strict recall (unconfirmed as misses) | 97.9% |
| Hits: selected, run everything, checks | 488, 727, 236 |
| Misses | 27 |
| Flaky, unconfirmed, unattributed | 57, 4, 35 |
| Unavailable, errors, ignored | 0, 0, 3 |
| Test files selected: median, p90 | 3.7%, 100.0% |
| Plans that selected everything | 18.3% |
| Plan time: first (cold), median, p90 | 1.62 s, 0.13 s, 0.53 s |
| Recall on `pull_request` runs (strict) | 98.2% (97.9%) |

Quarantined tests (declared in the bench config, applied only while the data bears them out):

- `packages/sql/mysql2/test/Persistence.test.ts` in jobs `^Test \(1/2, (Node Deno)\)$`: active, 21 failures absorbed (14 would-be hits, 7 would-be misses), 19 pull requests, until 2026-12-31. Times out after 30 s waiting on MySQL, on changes that touch neither: failed in 13 pull requests in July, only in the first test shard. Two of them broke the build instead (a transform error, a missing import).
  - Absorbed in: `Test (1/2, Deno)`, `Test (1/2, Node)`
- `packages/sql/mysql2/test/KeyValueStore.test.ts` in jobs `^Test \(2/2, (Node Deno)\)$`: active, 9 failures absorbed (8 would-be hits, 1 would-be misses), 9 pull requests, until 2026-12-31. Its setup hook times out after 30 s waiting on MySQL: failed in 8 pull requests in July, only in the second test shard. Two of them broke the build instead (a transform error, a missing import).
  - Absorbed in: `Test (2/2, Deno)`, `Test (2/2, Node)`
- `packages/sql/d1/test/Resolver.test.ts` in jobs `^Test \(2/2, Node\)$`: active, 7 failures absorbed (7 would-be hits, 0 would-be misses), 8 pull requests, until 2026-12-31. Times out after 5 s against the local D1 engine: failed in 7 pull requests in July, only in the second Node test shard.
  - Absorbed in: `Test (2/2, Node)`

Plans that selected everything, by cause:

- `unreached packages/**`: 23
- `run-all tsconfig.base.json`: 18
- `run-all package.json`: 13
- `run-all .github/**`: 12
- `run-all packages/**`: 5
- `unreached .agents/**`: 4
- `run-all vitest.config.ts`: 2
- `unreached .vscode/**`: 2
- `unreached ai-docs/**`: 2
- `unreached scripts/**`: 2

Failed jobs no rule watches:

- `AI Documentation Generation`: 3
- `Circular Dependencies`: 4
- `Test on Bun`: 4

Misses:

- run 29435902211 attempt 1: `packages/sql/pg/test/Client.test.ts`, changed `.changeset/cli-wizard-mode.md, packages/effect/src/unstable/cli/Command.ts, packages/effect/src/unstable/cli/GlobalFlag.ts, packages/effect/src/unstable/cli/Param.ts, packages/effect/src/unstable/cli/internal/ansi.ts`
- run 29544337445 attempt 2: `packages/effect/test/schema/toArbitrary.test.ts`, changed `.changeset/fix-cluster-mssql-for-update.md, packages/effect/src/unstable/cluster/SqlMessageStorage.ts, packages/platform-node/package.json, packages/platform-node/test/cluster/SqlMessageStorage.test.ts, packages/platform-node/test/cluster/SqlMessageStorageMssql.test.ts`
- run 30173861286 attempt 1: `packages/sql/libsql/test/Client.test.ts`, changed `.changeset/fix-otlp-exporter-shutdown.md, packages/effect/src/unstable/observability/OtlpExporter.ts, packages/effect/test/unstable/observability/OtlpExporter.test.ts`
- run 30312917536 attempt 1: `packages/platform-node/test/NodeRedis.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30312917536 attempt 1: `packages/platform-node/test/cluster/SqlRunnerStorage.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30312917536 attempt 1: `packages/sql/libsql/test/Client.test.ts`, changed `.changeset/expose-ai-prompt-part-schemas.md, packages/effect/src/unstable/ai/Prompt.ts, packages/effect/test/unstable/ai/Prompt.test.ts`
- run 30324294308 attempt 1: `packages/sql/libsql/test/Client.test.ts`, changed `.changeset/fresh-lines-wait.md, packages/platform-node-shared/src/NodeTerminal.ts, packages/platform-node-shared/test/NodeTerminal.test.ts, packages/platform-node-shared/test/fixtures/node-terminal.ts`
- run 30390581785 attempt 2: `packages/sql/libsql/test/Resolver.test.ts`, changed `.changeset/fix-reactive-query-metadata.md, packages/effect/src/unstable/reactivity/AtomHttpApi.ts, packages/effect/src/unstable/reactivity/AtomRpc.ts, packages/effect/test/reactivity/AtomHttpApi.test.ts, packages/effect/test/reactivity/AtomRpc.test.ts`
- run 30414439050 attempt 1: `packages/tools/openapi-generator/test/OpenApiGeneratorCli.test.ts`, changed `.changeset/add-deno-multipart.md, packages/platform-deno/src/DenoMultipart.ts, packages/platform-deno/src/index.ts`
- run 30414439050 attempt 1: `packages/tools/openapi-generator/test/JsonSchemaGeneratorRepresentation.test.ts`, changed `.changeset/add-deno-multipart.md, packages/platform-deno/src/DenoMultipart.ts, packages/platform-deno/src/index.ts`
- run 30690677178 attempt 1: `packages/sql/pg/test/Persistence.integration.test.ts`, changed `.changeset/mcp-protocol-versions.md, packages/effect/MCP.md, packages/effect/src/unstable/ai/McpProtocol.ts, packages/effect/src/unstable/ai/McpSchema.ts, packages/effect/src/unstable/ai/McpServer.ts`
- run 30711443368 attempt 1: `packages/sql/mysql2/test/Persistence.integration.test.ts`, changed `packages/effect/test/Channel.test.ts`
- run 30711795071 attempt 1: `packages/sql/pg/test/Client.integration.test.ts`, changed `packages/effect/test/SubscriptionRef.test.ts`
- run 30711795071 attempt 1: `packages/sql/mysql2/test/Persistence.integration.test.ts`, changed `packages/effect/test/SubscriptionRef.test.ts`
- run 30715081238 attempt 1: `packages/effect/test/schema/toArbitrary.test.ts`, changed `packages/effect/test/LayerMap.test.ts`
- run 30781854226 attempt 1: `packages/platform-node/test/NodeHttpClient.test.ts`, changed `.changeset/secure-eventlog-identities.md, packages/effect/src/unstable/eventlog/EventLogServer.ts, packages/effect/src/unstable/eventlog/EventLogServerEncrypted.ts, packages/effect/src/unstable/eventlog/EventLogServerUnencrypted.ts, packages/sql/sqlite-node/test/SqlEventLogServerEncrypted.test.ts`
- run 30788174592 attempt 1: `packages/platform-node/test/NodeHttpClient.test.ts`, changed `packages/effect/test/Chunk.test.ts`
- run 30899876531 attempt 1: `packages/platform-node/test/NodeRedis.integration.test.ts`, changed `packages/effect/test/unstable/cli/Help.test.ts`
- run 30899876531 attempt 1: `packages/sql/mysql2/test/KeyValueStore.integration.test.ts`, changed `packages/effect/test/unstable/cli/Help.test.ts`
- run 30899876531 attempt 1: `packages/platform-node/test/cluster/SqlRunnerStorage.integration.test.ts`, changed `packages/effect/test/unstable/cli/Help.test.ts`
- run 30899876531 attempt 1: `packages/sql/libsql/test/Client.integration.test.ts`, changed `packages/effect/test/unstable/cli/Help.test.ts`
- run 30955200961 attempt 1: `packages/platform-deno/test/cluster/SocketRunner.test.ts`, changed `.changeset/good-cups-reply.md, packages/sql/mssql/src/MssqlClient.ts, packages/sql/mssql/test/Client.test.ts`
- run 30970000436 attempt 1: `packages/sql/mysql2/test/Persistence.integration.test.ts`, changed `.changeset/add-httpapi-with-headers.md, packages/effect/HTTPAPI.md, packages/effect/src/unstable/http/HttpServerResponse.ts, packages/effect/src/unstable/httpapi/HttpApi.ts, packages/effect/src/unstable/httpapi/HttpApiBuilder.ts`
- run 31047937195 attempt 1: `packages/platform-node/test/cluster/SqlMessageStorage.integration.test.ts`, changed `packages/ai/openai/test/OpenAiSchema.test.ts`
- run 31048550066 attempt 1: `packages/sql/mysql2/test/KeyValueStore.integration.test.ts`, changed `packages/effect/test/Stream.test.ts`
- run 31173225965 attempt 1: `packages/sql/mysql2/test/Persistence.integration.test.ts`, changed `packages/ai/openai-compat/typetest/OpenAiTelemetry.tst.ts`
- run 31654685741 attempt 1: `packages/sql/mssql/test/Persistence.integration.test.ts`, changed `.changeset/scope-message-storage-clear-address.md, packages/effect/src/unstable/cluster/MessageStorage.ts, packages/effect/src/unstable/cluster/SqlMessageStorage.ts, packages/effect/test/cluster/MessageStorage.test.ts, packages/platform/node/test/cluster/MessageStorageTest.ts`

## pnpm/pnpm

Window 2026-05-12 to 2026-08-09; dataset `bench/data/pnpm_pnpm.jsonl`, 2500 run attempts from 2026-06-25 to 2026-08-09. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 381 |
| Attributed failures (gate 30) | 472 (met) |
| Recall | 99.6% |
| Strict recall (unconfirmed as misses) | 99.6% |
| Hits: selected, run everything, checks | 73, 397, 0 |
| Misses | 2 |
| Flaky, unconfirmed, unattributed | 9, 0, 221 |
| Unavailable, errors, ignored | 0, 0, 415 |
| Test files selected: median, p90 | 100.0%, 100.0% |
| Plans that selected everything | 84.4% |
| Plan time: first (cold), median, p90 | 1.40 s, 0.48 s, 0.55 s |
| Recall on `pull_request` runs (strict) | 99.6% (99.6%) |

Plans that selected everything, by cause:

- `unreached pnpm/**`: 112
- `run-all pnpm-lock.yaml`: 75
- `run-all .github/**`: 35
- `unreached Cargo.lock`: 30
- `unreached pacquet/**`: 15
- `unreached .gitignore`: 14
- `unreached cspell.json`: 9
- `run-all package.json`: 8
- `run-all pnpm-workspace.yaml`: 3
- `unreached .typos.toml`: 1

Misses:

- run 29268600635 attempt 1: `pnpm11/releasing/commands/test/change/index.test.ts`, changed `pnpm11/installing/deps-installer/test/install/verifyLockfileResolutionsCache.ts`
- run 31128168747 attempt 1: `pnpm11/deps/status/test/checkDepsStatus.test.ts`, changed `.changeset/git-ci-https-fallback.md, pnpm11/resolving/git-resolver/src/parseBareSpecifier.ts, pnpm11/resolving/git-resolver/test/index.ts`

## vitest-dev/vitest

Window 2026-07-02 to 2026-09-29; dataset `bench/data/vitest-dev_vitest.jsonl`, 1661 run attempts from 2026-06-21 to 2026-09-29. Fetch: `fetch complete: 8 new rows in out/data.jsonl`.

| Measure | Value |
| --- | --- |
| Runs replayed | 1104 |
| Attributed failures (gate 30) | 660 (met) |
| Recall | 95.9% |
| Recall with quarantined tests counted (raw) | 93.6% (n=857) |
| Recall with them left out (adjusted) | 95.9% (n=660) |
| Strict recall (unconfirmed as misses) | 94.6% |
| Hits: selected, run everything, checks | 525, 51, 57 |
| Misses | 27 |
| Flaky, unconfirmed, unattributed | 7, 9, 3280 |
| Unavailable, errors, ignored | 0, 0, 1664 |
| Test files selected: median, p90 | 98.7%, 100.0% |
| Plans that selected everything | 30.7% |
| Plan time: first (cold), median, p90 | 0.93 s, 0.13 s, 0.25 s |
| Recall on `pull_request` runs (strict) | 95.9% (94.6%) |

Quarantined tests (declared in the bench config, applied only while the data bears them out):

- `test/typescript/test/typechecker.test.ts` in jobs `^Test: unit, node-24, windows-latest$`: active, 100 failures absorbed (82 would-be hits, 17 would-be misses), 54 pull requests, until 2026-12-31. Fails only on the Windows unit job (its out-of-memory crash and missing-command cases): 60 failures across 36 pull requests from June to August 2026, while the same job passed 356 times and no other job ever failed it.
  - Absorbed in: `Test: unit, node-24, windows-latest`

Plans that selected everything, by cause:

- `run-all .github/**`: 186
- `run-all package.json`: 61
- `run-all pnpm-lock.yaml`: 54
- `run-all pnpm-workspace.yaml`: 9
- `run-all test/**`: 8
- `unreached .dockerignore`: 8
- `unreached eslint.config.js`: 4
- `run-all examples/**`: 2
- `unreached .gitignore`: 2
- `unreached knip.jsonc`: 2

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

- run 33165835216 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/browser-playwright/src/playwright.ts`
- run 33178368623 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/browser-playwright/src/playwright.ts`
- run 33242987323 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/FileDetails.vue, packages/ui/client/components/views/ViewReport.spec.ts, packages/ui/client/components/views/ViewReport.vue, test/ui/test/ui.spec.ts`
- run 33357536862 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/views/ViewEditor.vue, packages/ui/client/components/views/ViewTestReport.vue, packages/ui/client/composables/attachments.ts, test/ui/fixtures/playwright-trace/basic.test.ts, test/ui/fixtures/playwright-trace/vitest.config.ts`
- run 33463651093 attempt 1: `test/browser/specs/trace.test.ts`, changed `packages/coverage-v8/src/provider.ts, test/coverage-test/fixtures/configs/vitest.config.non-file-urls.ts, test/coverage-test/fixtures/test/non-file-urls-fixture.test.ts, test/coverage-test/test/non-file-urls.v8.test.ts, test/coverage-test/vitest.config.ts`
- run 33728541156 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/explorer/Explorer.vue, test/ui/fixtures/main/node/skipped.test.ts, test/ui/test/helper.ts, test/ui/test/ui.spec.ts`
- run 33728612549 attempt 1: `test/browser/specs/to-match-screenshot.test.ts`, changed `packages/ui/client/components/explorer/Explorer.vue, packages/ui/client/composables/explorer/collector.ts, test/ui/fixtures/main/node/skipped.test.ts, test/ui/test/helper.ts, test/ui/test/ui.spec.ts`
- run 34188350742 attempt 1: `test/browser/specs/bail-out.test.ts`, changed `packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/composables/trace-view.ts, packages/ui/client/pages/index.vue`
- run 34195425889 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/trace/TraceArtifacts.vue, packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/composables/trace-view.ts`
- run 34297410532 attempt 1: `test/browser/specs/locators.test.ts`, changed `packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/pages/index.vue, test/ui/test/trace.spec.ts`
- run 34297410532 attempt 1: `test/browser/specs/server-url.test.ts`, changed `packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/pages/index.vue, test/ui/test/trace.spec.ts`
- run 34299387859 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/pages/index.vue, test/ui/test/trace.spec.ts`
- run 34299387859 attempt 1: `test/e2e/test/list.test.ts`, changed `packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/pages/index.vue, test/ui/test/trace.spec.ts`
- run 34299387859 attempt 1: `test/e2e/test/open-telemetry.test.ts`, changed `packages/ui/client/components/trace/TraceViewPane.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts, packages/ui/client/pages/index.vue, test/ui/test/trace.spec.ts`
- run 34458638078 attempt 1: `test/coverage-test/test/reporters.test.ts`, changed `packages/ui/client/components/BrowserIframe.vue, packages/ui/client/styles/main.css`
- run 34812943967 attempt 1: `test/browser/specs/mocking.test.ts`, changed `packages/ui/client/composables/explorer/filter.ts, test/ui/test/ui.spec.ts`
- run 34819873999 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/composables/explorer/expand.ts, packages/ui/client/composables/explorer/filter.ts, packages/ui/client/composables/explorer/state.ts, test/ui/test/ui.spec.ts`
- run 34828714090 attempt 1: `test/coverage-test/test/include-exclude.test.ts`, changed `packages/ui/client/composables/explorer/expand.ts, packages/ui/client/composables/explorer/filter.ts, packages/ui/client/composables/explorer/state.ts, test/ui/test/ui.spec.ts`
- run 34920150411 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/composables/explorer/collapse.ts, packages/ui/client/composables/explorer/collector.ts, packages/ui/client/composables/explorer/expand.ts, packages/ui/client/composables/explorer/filter.ts`
- run 34956251133 attempt 1: `test/browser/specs/mocking.test.ts`, changed `packages/ui/client/pages/index.vue, packages/ui/client/styles/main.css`
- run 34956251133 attempt 1: `test/browser/specs/oxc.test.ts`, changed `packages/ui/client/pages/index.vue, packages/ui/client/styles/main.css`
- run 34979372095 attempt 1: `test/browser/specs/runner.test.ts`, changed `test/e2e/test/reporters/configuration-options.test-d.ts`
- run 34979372095 attempt 1: `test/browser/specs/trace.test.ts`, changed `test/e2e/test/reporters/configuration-options.test-d.ts`
- run 35403972579 attempt 1: `test/coverage-test/test/include-exclude.test.ts`, changed `CLAUDE.md`
- run 35604155400 attempt 1: `test/unit/test/expect-poll.test.ts`, changed `test/unit/test/jest-expect.test.ts`
- run 36437685026 attempt 1: `test/browser/specs/aria-snapshot.test.ts`, changed `packages/ui/client/components/BrowserIframe.vue, packages/ui/client/styles/main.css, test/browser/specs/projects-ui.test.ts, test/browser/specs/ui.test.ts`
- run 36437685026 attempt 1: `test/browser/specs/playwright-trace.test.ts`, changed `packages/ui/client/components/BrowserIframe.vue, packages/ui/client/styles/main.css, test/browser/specs/projects-ui.test.ts, test/browser/specs/ui.test.ts`
