# Benchmarks

Replay results on public repositories, written by the `bench` workflow. See [Replay](replay.md) for what each measure means; the configs are in `bench/`.

Planned with Fairlead 0.1.1 at `8c1b179d44cc`.

## Effect-TS/effect

Window 2026-05-09 to 2026-08-06; dataset `bench/data/Effect-TS_effect.jsonl`, 1000 run attempts from 2026-06-22 to 2026-08-06. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 139 |
| Attributed failures (gate 30) | 853 (met) |
| Recall | 98.8% |
| Recall with quarantined tests counted (raw) | 98.0% (n=890) |
| Recall with them left out (adjusted) | 98.8% (n=853) |
| Strict recall (unconfirmed as misses) | 98.8% |
| Hits: selected, run everything, checks | 64, 703, 76 |
| Misses | 10 |
| Flaky, unconfirmed, unattributed | 30, 0, 21 |
| Unavailable, errors, ignored | 0, 0, 2 |
| Test files selected: median, p90 | 98.6%, 100.0% |
| Plans that selected everything | 31.7% |
| Plan time: first (cold), median, p90 | 1.67 s, 0.20 s, 1.25 s |
| Recall on `pull_request` runs (strict) | 98.8% (98.8%) |

Quarantined tests (declared in the bench config, applied only while the data bears them out):

- `packages/sql/mysql2/test/Persistence.test.ts` in jobs `^Test \(1/2, (Node Deno)\)$`: active, 21 failures absorbed (14 would-be hits, 7 would-be misses), 19 pull requests, until 2026-12-31. Times out after 30 s waiting on MySQL, on changes that touch neither: failed in 13 pull requests in July, only in the first test shard. Two of them broke the build instead (a transform error, a missing import).
  - Absorbed in: `Test (1/2, Deno)`, `Test (1/2, Node)`
- `packages/sql/mysql2/test/KeyValueStore.test.ts` in jobs `^Test \(2/2, (Node Deno)\)$`: active, 9 failures absorbed (8 would-be hits, 1 would-be misses), 9 pull requests, until 2026-12-31. Its setup hook times out after 30 s waiting on MySQL: failed in 8 pull requests in July, only in the second test shard. Two of them broke the build instead (a transform error, a missing import).
  - Absorbed in: `Test (2/2, Deno)`, `Test (2/2, Node)`
- `packages/sql/d1/test/Resolver.test.ts` in jobs `^Test \(2/2, Node\)$`: active, 7 failures absorbed (7 would-be hits, 0 would-be misses), 7 pull requests, until 2026-12-31. Times out after 5 s against the local D1 engine: failed in 7 pull requests in July, only in the second Node test shard.
  - Absorbed in: `Test (2/2, Node)`

Plans that selected everything, by cause:

- `run-all package.json`: 12
- `unreached packages/**`: 11
- `run-all .github/**`: 9
- `run-all packages/**`: 4
- `run-all vitest.config.ts`: 2
- `unreached .vscode/**`: 2
- `unreached scripts/**`: 2
- `run-all pnpm-lock.yaml`: 1
- `run-all tsconfig.base.json`: 1

Failed jobs no rule watches:

- `AI Documentation Generation`: 2
- `Circular Dependencies`: 3
- `Test on Bun`: 1

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

## pnpm/pnpm

Window 2026-04-25 to 2026-07-23; dataset `bench/data/pnpm_pnpm.jsonl`, 1300 run attempts from 2026-06-25 to 2026-07-23. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 231 |
| Attributed failures (gate 30) | 312 (met) |
| Recall | 99.4% |
| Strict recall (unconfirmed as misses) | 99.4% |
| Hits: selected, run everything, checks | 43, 267, 0 |
| Misses | 2 |
| Flaky, unconfirmed, unattributed | 3, 0, 143 |
| Unavailable, errors, ignored | 0, 0, 240 |
| Test files selected: median, p90 | 100.0%, 100.0% |
| Plans that selected everything | 84.8% |
| Plan time: first (cold), median, p90 | 1.82 s, 0.48 s, 0.64 s |
| Recall on `pull_request` runs (strict) | 99.4% (99.4%) |

Plans that selected everything, by cause:

- `unreached pnpm/**`: 61
- `run-all pnpm-lock.yaml`: 42
- `unreached Cargo.lock`: 27
- `run-all .github/**`: 20
- `unreached pacquet/**`: 15
- `unreached .gitignore`: 14
- `run-all package.json`: 6
- `unreached cspell.json`: 4
- `unreached .typos.toml`: 1

Misses:

- run 29268600635 attempt 1: `pnpm11/releasing/commands/test/change/index.test.ts`, changed `pnpm11/installing/deps-installer/test/install/verifyLockfileResolutionsCache.ts`
- run 29268600635 attempt 1: `pnpm11/releasing/commands/test/change/index.test.ts`, changed `pnpm11/installing/deps-installer/test/install/verifyLockfileResolutionsCache.ts`

## vitest-dev/vitest

Window 2026-06-13 to 2026-09-10; dataset `bench/data/vitest-dev_vitest.jsonl`, 1277 run attempts from 2026-06-21 to 2026-09-10. Fetch: `fetch partial: 300 new rows in out/data.jsonl; run again to continue`.

| Measure | Value |
| --- | --- |
| Runs replayed | 986 |
| Attributed failures (gate 30) | 528 (met) |
| Recall | 95.6% |
| Recall with quarantined tests counted (raw) | 93.6% (n=627) |
| Recall with them left out (adjusted) | 95.6% (n=528) |
| Strict recall (unconfirmed as misses) | 95.3% |
| Hits: selected, run everything, checks | 412, 44, 49 |
| Misses | 23 |
| Flaky, unconfirmed, unattributed | 4, 2, 3369 |
| Unavailable, errors, ignored | 0, 0, 878 |
| Test files selected: median, p90 | 98.7%, 100.0% |
| Plans that selected everything | 34.9% |
| Plan time: first (cold), median, p90 | 1.37 s, 0.19 s, 0.33 s |
| Recall on `pull_request` runs (strict) | 95.6% (95.3%) |

Quarantined tests (declared in the bench config, applied only while the data bears them out):

- `test/typescript/test/typechecker.test.ts` in jobs `^Test: unit, node-24, windows-latest$`: active, 100 failures absorbed (82 would-be hits, 17 would-be misses), 54 pull requests, until 2026-12-31. Fails only on the Windows unit job (its out-of-memory crash and missing-command cases): 60 failures across 36 pull requests from June to August 2026, while the same job passed 356 times and no other job ever failed it.
  - Absorbed in: `Test: unit, node-24, windows-latest`

Plans that selected everything, by cause:

- `run-all .github/**`: 145
- `run-all pnpm-lock.yaml`: 79
- `run-all package.json`: 50
- `unreached docs/**`: 34
- `run-all test/**`: 8
- `unreached .dockerignore`: 8
- `run-all pnpm-workspace.yaml`: 5
- `unreached eslint.config.js`: 3
- `run-all examples/**`: 2
- `unreached .gitignore`: 2

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

- run 33070884330 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/coverage-v8/src/provider.ts, test/coverage-test/fixtures/configs/vitest.config.non-file-urls.ts, test/coverage-test/fixtures/test/non-file-urls-fixture.test.ts, test/coverage-test/test/non-file-urls.v8.test.ts`
- run 33070884330 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/coverage-v8/src/provider.ts, test/coverage-test/fixtures/configs/vitest.config.non-file-urls.ts, test/coverage-test/fixtures/test/non-file-urls-fixture.test.ts, test/coverage-test/test/non-file-urls.v8.test.ts`
- run 33076437839 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/coverage-v8/src/provider.ts, test/coverage-test/test/non-file-urls.unit.test.ts`
- run 33131338230 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/ui/client/components/trace/TraceView.vue, packages/ui/client/composables/navigation.ts, test/ui/test/trace.spec.ts`
- run 33165835216 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/browser-playwright/src/playwright.ts`
- run 33178368623 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/browser-playwright/src/playwright.ts`
- run 33242987323 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/FileDetails.vue, packages/ui/client/components/views/ViewReport.spec.ts, packages/ui/client/components/views/ViewReport.vue, test/ui/test/ui.spec.ts`
- run 33242987323 attempt 1: `test/e2e/test/detect-async-leaks.test.ts`, changed `packages/ui/client/components/FileDetails.vue, packages/ui/client/components/views/ViewReport.spec.ts, packages/ui/client/components/views/ViewReport.vue, test/ui/test/ui.spec.ts`
- run 33345329729 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/coverage-v8/src/index.ts, packages/coverage-v8/src/provider.ts, test/coverage-test/fixtures/configs/vitest.config.non-file-urls.ts, test/coverage-test/fixtures/test/non-file-urls-fixture.test.ts, test/coverage-test/test/non-file-urls.v8.test.ts`
- run 33357536862 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/views/ViewEditor.vue, packages/ui/client/components/views/ViewTestReport.vue, packages/ui/client/composables/attachments.ts, test/ui/fixtures/playwright-trace/basic.test.ts, test/ui/fixtures/playwright-trace/vitest.config.ts`
- run 33365566579 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/FileDetails.vue, packages/ui/client/components/views/ViewReport.spec.ts, packages/ui/client/components/views/ViewReport.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts`
- run 33366617422 attempt 1: `test/browser/specs/runner.test.ts`, changed `packages/ui/client/components/FileDetails.vue, packages/ui/client/components/views/ViewReport.spec.ts, packages/ui/client/components/views/ViewReport.vue, packages/ui/client/composables/navigation.ts, packages/ui/client/composables/params.ts`
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
