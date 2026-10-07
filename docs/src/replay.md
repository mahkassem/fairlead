# Replay

Replay is how Fairlead proves a plan's recall: it takes real CI failures from a repository's history, plans each failing commit again, and counts every failing test file or check the plan would have left out. It runs in two phases, so the report can always be reproduced from committed data.

```sh
fairlead replay fetch --repo owner/name --since 2026-07-01 --workflow ci.yml --data bench/data/owner_name.jsonl --clone ../name
fairlead replay run --data bench/data/owner_name.jsonl --clone ../name --config bench/owner_name.toml --fetch-missing
```

## `replay fetch`

Lists every completed `pull_request` and `merge_group` run since `--since`, a week at a time since one listing returns at most 1,000 runs, and records one row per run attempt, passing or failing, so a job that failed and then passed on a re-run shows up as flaky. `--workflow FILE` (repeatable; a workflow's file name, such as `ci.yml`, or its id) lists only those workflows' runs, so other workflows cost no requests, and a first attempt that was cancelled or skipped is left out: it ran nothing, and each recorded attempt costs an API request. It needs `curl` and a token in `GITHUB_TOKEN` or `GH_TOKEN`; in GitHub Actions the workflow's own token can read other public repositories' runs and logs.

`--event push` also records the default branch's completed push runs. Once pull requests run only the planned tests, a pull request run can't show a miss, since only the plan ran; the evidence is on the default branch, which usually still runs everything after a merge. A test that fails there and that the merged change's plan left out got past the plan: an escape.

*Since 0.9.0:* `--event schedule` records the default branch's completed scheduled runs, for a project whose pushes run the plan too and whose full suite runs on a schedule (a stage-`full` nightly, say). There, an escape is a test the nightly caught that no push's plan reached. See [Scheduled runs](#scheduled-runs).

- **Failed jobs keep the extractor's input,** not its output: failure-level annotations (GitHub's `.github` exit-code note left out) and the log lines around each `FAIL`, `●` or bun `(fail)`, with the bun file header each failure sits under, each PHPUnit failure or Pest `FAILED` line with the stack frames after it, under `go test -v` the lines a failed test logged before its `--- FAIL`, and Surefire's `<<< ERROR!` lines and closing lists, the Gradle task header each `FAILED` line sits under, Gradle's failed-task lines and Kotlin and Java compile errors. Logs expire after 90 days; the dataset keeps enough to re-extract with a better extractor later.
- **Each row records its pull request and base.** For a pull request run, the pull request comes from the run, the commit's associated pulls, or, for a fork, the pulls whose head is the fork's branch; the base is the base branch's first-parent commit when the run started, or the default branch's when no pull request is found. For a merge queue run, both come from the queue branch's name (`gh-readonly-queue/<base>/pr-<N>-<sha>`). For a push, the pull request is the one the commit's associated pulls say merged it (the first one when none does, none for a direct commit), and the base is the pushed commit's first parent, the branch before the merge; without `--clone` it's left for `replay run` to read. A scheduled run records neither: its base is the last green scheduled run, which `replay run` finds among the rows.
- **With `--clone`,** each run's head commit is fetched into the clone, so it's there when `replay run` needs it.
- **Rows already in the dataset are skipped** before any of their jobs are fetched, so a weekly fetch is incremental. `--limit N` stops after N new attempts.
- **GitHub's secondary rate limit,** which refuses bursts with a 403 or 429 however much of the hourly budget is left, is waited out, for API calls and log downloads alike: up to three retries, after the `Retry-After` GitHub sends or one, two and three minutes. Each wait prints a line. A 500, 502, 503 or 504 is GitHub failing for a moment and is retried too, after 5, 10 and 15 seconds. An exhausted hourly budget (`X-RateLimit-Remaining: 0`), any other refusal, or a server error that persists stops the fetch with GitHub's message; the rows fetched so far are kept.
- **The last line says how it ended:** `fetch complete`, `fetch partial` (the limit was reached; run it again to continue), or `fetch stopped` with the API's error and exit code 2. The rows gathered so far are written in every case, and a report from a partial dataset shouldn't be read as the whole window.
- **The token never reaches curl's arguments,** only its standard input, and a redirect to log storage is followed without it.

The dataset is JSON lines, appended and never rewritten.

## `replay run`

With `--fetch-missing`, the recorded heads and bases the clone lacks are fetched first. `--json-out PATH` writes the JSON report as well as printing the text one. While it works it prints progress on stderr, as plain lines a CI log keeps: every 25 planned runs and at least once a minute (`412 of 806 runs planned · 93 failures judged · 3 misses · 10 min, about 10 min left`), and a line for any run that takes over 10 seconds to plan and judge. `--quiet` turns that off; the report on stdout and in `--json-out` is the same either way. For each failed row in the window, replay checks out the head commit into a worktree beside the clone (created once and reused, so the [parse cache](graph.md) carries over), plans the change from the merge base of the recorded base (or, for a row without one, the clone's default branch as it stood when the run started) and the head, and classes every failure the row names:

| Outcome | Meaning |
| --- | --- |
| hit | the test file or check is in the plan, or the plan selects everything |
| miss | it isn't, and nothing below explains it; recall is hits over hits and misses |
| flaky | another attempt of the same run passed the same job; decided before hit, so a flaky failure never raises recall |
| unconfirmed | a later run of the same change passed the job: the same head again, or the same patch rebased onto a newer base (equal `git patch-id`) |
| unattributed | the job failed, but no test file or check could be named from its annotations or log |
| unavailable | the head commit, or the history to its merge base, isn't in the clone |
| error | the planner refused the commit, such as a test file no runner matches |
| unwatched | no `[[replay.failures]]` or `[[replay.checks]]` entry names the job; listed by job name so a gap in the config can't raise recall |
| quarantined | a `[[replay.quarantine]]` entry declares the test flaky in this job (below) |
| environment | the job failed alike across unrelated pull requests once its runner image changed (below) |
| inherited | the failure came with the base branch: the base's own push run failed the test in the job, or three or more unrelated pull requests on the same base did (below) |
| ignored | `replay.ignore` names the job, such as one that only aggregates others, or it failed only in steps `replay.ignore_steps` names, such as an install |

Recall is hits over hits and misses. Strict recall also counts unconfirmed failures as misses.

Failing test files come from annotations and from the log, through the extractors named in `[[replay.failures]]`: `vitest`, `jest`, `bun`, `phpunit`, `pest` (which also reads Laravel's `php artisan test`, and PHPUnit's format, which `artisan test --parallel` and Pest's own closing section print), `go` (`go test`, plain or `-v`, and gotestsum), `pytest` (its `FAILED` and `ERROR` summary lines, `-v` and xdist output, and modules that failed to import), `maven` and `gradle` (since 0.7.0, below), or `regex` with a pattern that has a named `file` group (and optionally `project` and `title`). PHPUnit and Pest name a failing test by its class, so the test file is the stack frame or `at` path whose file name is the class's, and otherwise the class name as a path: an error thrown in the code under test doesn't pass for the test file. A path on the runner, such as `/home/runner/work/app/app/tests/Unit/FooTest.php`, is matched by its longest tail that is a file in the repository. `go test` names a failed test's file without its directory, so it's printed under the package's import path, `/example.com/app/cart/cart_test.go`, and matched the same way, or, when the module sits in a subfolder, by the longest tail of two segments or more that ends exactly one file. The file is the one the test logged, else a test file in its panic, else a compile error in a test file; a failure with none of these is unattributed. A printed path is matched to a file at that commit: as it stands, under the named project or package, by a unique suffix, or among the suffix matches by the one whose source contains the failing test's title. Anything still ambiguous is unattributed. Failing checks come from `[[replay.checks]]`, which map a job and step name to a check id.

*Since 0.7.0:* `maven` reads Maven's Surefire and Failsafe, and `gradle` reads Gradle's test output, plain or with GitHub's timestamps. Both name a failing test by its class, so the class becomes a path and the file is the Java or Kotlin one whose path ends with it, under any module's source root: `com.acme.FooTest` is `…/com/acme/FooTest.java` or `.kt`. A nested class, `Outer$Inner` or `Outer.Inner`, is its outer class's file. Kotlin may leave a package's leading folders out, so failing that, a shorter tail counts for a file that declares the class's package.

- **Maven:** each failed test's `[ERROR] com.acme.FooTest.method -- Time elapsed: … <<< FAILURE!` (or `<<< ERROR!`; `method(com.acme.FooTest)` before Surefire 3), and a class whose `Tests run: … <<< FAILURE! -- in com.acme.FooTest` line names no test, such as one that failed in `@BeforeAll`. The closing `[ERROR] Failures:` and `Errors:` lists repeat these by simple name, so an entry counts only for a class nothing above named. Flaky tests, which Surefire prints as warnings, don't count.
- **Gradle:** `FooTest > method() FAILED`, with the class's simple name under JUnit 5 and its qualified name under JUnit 4, and the nested class or parameterized case between them (`FooTest > Nested > method() > [1] a FAILED`). The project is the one in the `> Task :module:test` header the line sits under, which Gradle's grouped output prints again whenever another task's output follows, so `--parallel` builds attribute too. When several modules hold a class of that name, the one under the project's folder is the file, and after that the one holding the test's name. A task whose `There were failing tests` report follows no `FAILED` line, as under `--quiet`, is unattributed. A task that compiles tests, such as `compileTestKotlin` or `compileTestJava`, fails on the file that doesn't compile: its `e: file.kt:44:13` or `File.java:31: error:` line names the test file. A compile error in main code is no test's failure, so it stays unattributed.

The window is the `replay.window_days` days ending at the newest recorded run, or at `--until`, so the same dataset always gives the same report. The report says whether `replay.min_failures` attributed failures were reached.

```toml
[replay]
window_days = 90
min_failures = 30
ignore = ["^all-green$"]

[[replay.failures]]
runner = "vitest"
extractor = "vitest"
job = "^test"

[[replay.checks]]
job = "^lint$"
step = "^Typecheck$"
check = "typecheck"
```

## Inherited failures

A pull request can fail a test only because its base branch already did. Replay groups failures by base commit, job and test file, and calls a group inherited when either holds:

- **The base's own run failed it.** A push run recorded with `--event push` whose commit is the base failed the same test in the same job. That's direct proof, so one pull request is enough.
- **Unrelated pull requests failed it alike.** Pull requests on the same base failed it, and three or more of them changed no file in common with each other. A stack of related changes shares a cause, so it counts once.

Either way, a pull request that changed the test, or a file beside it that isn't another test (a fixture, a helper), stays out of the group: its own change may be what broke it. Hits and misses are grouped alike, so the rule can't be chosen to raise recall. An inherited failure keeps what it would have been; it's left out of adjusted recall and the `min_failures` gate and counted in raw recall, as quarantined ones are. The report lists each group with its base, job, test, pull requests and evidence, and says "fixed later" when a later run on another base passed the job, else "unresolved".

## Runner image waves

When a floating runner label such as `ubuntu-latest` moves to a new image, a job can fail on every pull request for days until someone fixes it, whatever each one changed. `replay fetch` records each failed job's runner image and version from its log's "Runner Image" header, and `replay run` calls a group of failures a wave when all of these hold:

- the same test failed in the same job on an image version that job's failures hadn't run on before;
- within 7 days of that version first appearing;
- in three or more pull requests that changed no file in common, none of them touching the test or a non-test file beside it;
- and the test hadn't failed in that job on an older image, so it isn't an ordinary regression that happens to share the dates.

Wave failures get the outcome `environment`: kept out of adjusted recall and counted in raw recall, hits and misses alike, like inherited ones. The report lists each wave with its job, test, old and new image, when the new one appeared, and its pull requests. Only failed jobs' logs are read, so an image is known only from failures, and datasets fetched before this have none.

## Quarantine

A test that fails in one CI job whatever changes, such as a platform-specific flake, measures that job rather than the planner. A `[[replay.quarantine]]` entry declares it:

```toml
[[replay.quarantine]]
path = "test/typecheck.test.ts"     # one test file
job = "^unit, windows$"             # the jobs it fails in, as a regex
reason = "Fails only on Windows: 60 failures across 36 pull requests; the job passed 356 times."
until = "2026-12-31"                # it stops applying after this day
```

Every failure is still planned and judged. While an entry applies, its matching hits, misses and unconfirmed failures become `quarantined`, and the report says what each would have been. It applies only while the dataset bears it out: the test failed in at least three distinct pull requests in the window, and never in a job the entry doesn't name. Otherwise it's reported `unverified`, with the other jobs named. After `until` it's `expired`, and when the test didn't fail in the window, or only flakily, it's `stale`, a sign the entry can go. `until` is compared with the end of the window, the dataset's newest run unless `--until` says otherwise, not with today, so a replay gives the same answer whenever it runs. The report names the jobs each entry absorbed failures in, which shows a regex that reaches further than meant. The report gives recall both ways, with the number of failures each is measured on: raw (quarantined failures counted as what they would have been) and adjusted (left out). Quarantined failures don't count toward `replay.min_failures`.

## Scheduled runs

*Since 0.9.0:* a failing scheduled run at commit C is judged job by job. For each failed job, P is the newest earlier scheduled run of the same workflow in the window where that job passed, the same "last green" `ci plan --since-green` uses. The run is planned from P to C, so the plan covers every push in between, and each failing test is classed against it:

| Outcome | When |
| --- | --- |
| hit | the plan from P to C selects it: a push's change reaches it |
| escape | the plan leaves it out, so no push in P..C ran it |
| unconfirmed | a later scheduled run passed the job on the same tree, or P's tree is C's (nothing changed, so the change can't be what failed) |
| unavailable | no earlier scheduled run in the window passed the job, or P's commit isn't in the clone |

An escape names where it came in: the push, and the pull request its recorded push run names, when P..C holds exactly one push; otherwise the range and how many pushes it holds. Scheduled runs have their own report line, `schedule (full runs)`, with their misses counted as escapes, and each escape lists an `introduced` line.

To record them, add `--event schedule` to `replay fetch` (beside `--event push` if the pushes' runs are wanted too, which also lets an escape name its pull request). The window has to reach back to a green night: a job that has failed every night since the window began is unavailable until one passes.

## The report

Per repository: runs replayed, attributed failures against the gate, recall and strict recall (overall and per event, since merge queue runs usually run everything and are the better evidence), hits by how they were selected (by the plan, because the plan selected everything, or as a check), the other outcomes, unwatched jobs by name, the median and 90th-percentile share of test files selected, the share of plans that selected everything, and the first (cold), median and 90th-percentile planning time. Each miss lists the changed files and an owner rule that would have caught it. Push runs have their own line, `push (after merge)`, and scheduled runs theirs, `schedule (full runs)` (*since 0.9.0*), with their misses counted as escapes; for a repository whose pull requests run the plan, that line is the recall that matters, and for one whose pushes run the plan too, the scheduled line is. `--json` prints the same as JSON.

## Limits

- CI ran a pull request's merge commit with its base, while replay plans the head against the merge base. A failure caused by the base alone can show up as a miss, unless it's recognised as inherited (above): with the base's push runs recorded, or when enough unrelated pull requests share it.
- A repository whose pull request CI already runs only affected tests records only what it chose to run; its merge queue runs or its default branch's pushes (`--event push`), which usually run everything, are the better evidence, and "passed at a later push" is weaker there.
- A push is planned against its first parent, the diff the pull request's plan saw at merge time, and a failure there is unconfirmed only when a later push of the same tree passed the job (a revert and reland). A failure that appears only because two pull requests, each green alone, conflict shows as an escape on the second one.
- A scheduled run is planned once, from the last green run to its commit, not once per push. That plan covers the same changes as the pushes' plans together, except a change made and undone inside the range, which can't be what failed at C. An escape spread over several pushes names the range, not the push: telling which one brought it in would mean running the test at each.

## The benchmarks

`bench/` holds a config for each public benchmark repository, written from how that repository's CI runs its tests. The `bench` workflow runs weekly, or by hand with `since` (85 days back by default, since GitHub keeps job logs for 90) and `limit`:

- For each repository in turn, it builds Fairlead from the workflow's commit, starts from the dataset on the `bench-data` branch (or `main`), clones the repository with full history and no blobs, records new runs, and replays the window with `--fetch-missing`.
- A last job copies each grown dataset into `bench/data/`, writes [Benchmarks](benchmarks.md), and force-pushes both to `bench-data`, recreated from `main` each time, then opens a pull request from it if none is open. With a GitHub App configured (the `BENCH_APP_CLIENT_ID` variable and `BENCH_APP_PRIVATE_KEY` secret, the app allowed to write contents and pull requests), it pushes and opens the pull request as that app, so the pull request's checks start on their own. Without one it uses the workflow's token, and a maintainer approves the checks.
- The workflow token's API budget is about 1,000 requests an hour for the whole run, so a first backfill takes several runs: each records up to `limit` attempts per repository and the next carries on. A `BENCH_READ_TOKEN` secret, if set, is used instead. Pull requests opened by the workflow token don't start CI.

## Probing a repository

Before a repository joins the benchmark, the `replay-probe` workflow measures it once: dispatched with a repository, its CI workflow file and a config under `bench/probe/`, it records up to `limit` recent run attempts, replays them against that commit's planner and prints the report in the run's summary. It publishes nothing and keeps nothing between runs, so it's the way to try a new language or framework on real CI history.
