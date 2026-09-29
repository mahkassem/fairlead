# Import graph

Fairlead builds a file-level graph of what imports what, and later milestones plan tests from it. You can inspect it yourself:

```bash
fairlead graph stats                 # files, edges, what didn't resolve, build time
fairlead graph why FROM TO           # the chain by which FROM depends on TO
fairlead graph importers FILE        # the files that depend on FILE directly
```

## What counts as an edge

| Edge | From |
| --- | --- |
| import | `import`, `export … from`, `import x = require(…)` |
| type import | `import type` and `export type … from`; off with `graph.type_imports = false` |
| dynamic | `import("…")` with a plain string |
| require | `require("…")` and `require.resolve("…")` |
| mock | `vi.mock`, `jest.mock` and their relatives, naming a module |
| path literal | any string literal that names a file in the repository, so a test that spawns a CLI by its path depends on it |
| snapshot | `__snapshots__/x.test.ts.snap` to `x.test.ts` |
| rule | a `[[graph.edges]]` rule, below |

Single-file components are sources too. In a `.vue` or `.svelte` file every `<script>` block is read, `<script setup>` and Svelte's `context="module"` included, and in an `.astro` file its `---` frontmatter and its `<script>` blocks. Each block is parsed as its `lang` says (`ts`, `tsx`, `js`), else as JavaScript in Vue and Svelte and TypeScript in Astro, and a `<script src="…">` counts as an import. The component file is the node, so `import Card from './Card.vue'` resolves to it like any other import, and a change to a component reaches the tests that import it, directly or through other components.

An `import()` or `require()` whose argument isn't a plain string is counted as an unknown dynamic import; the planner treats the file as depending on its whole module.

## Edges the imports don't show

Some tests reach their code without importing it. An API test boots the server and calls it over HTTP, so no import joins the test to the area it exercises. A rule adds that edge, and the planner follows it like an import:

```toml
[[graph.edges]]
from = "apps/api/test/api/{area}{,-*}.test.ts"
to = ["apps/api/src/{area}/**"]
```

Each file matching `from` depends on every file its `to` globs match. A `{name}` stands for one path segment, with the same value on both sides, so `orders.test.ts` and `orders-refunds.test.ts` depend on `src/orders/`. Its value is read from the side where it's a whole segment: `{area}*` alone would also take `orders-refunds` as the name. `config check` refuses a `{name}` that is never a whole segment, or a `to` glob that names different ones than `from` (a `to` with none links to the same files for every match). `graph stats` counts rule edges, and names any rule that linked no file.

## PHP

`.php` files are scanned too (Blade templates, `.blade.php`, are left as plain files), and `graph stats` lists them as the `php` provider. PHP refers to classes by name, not by path, so each reference is first made fully qualified the way PHP does it: through the file's `namespace`, its `use` imports (grouped and aliased ones included), and `\` or `namespace\` prefixes. A file depends on:

- each class it names in a type, `new`, `::`, `extends`, `implements`, a trait `use`, an attribute or `instanceof`, and each name it imports with `use`;
- each function it calls by name, tried in its namespace and then globally, as PHP does, so a call to a helper reaches the file that declares it;
- each file an `include` or `require` names with a literal: `'x.php'`, `__DIR__ . '/x.php'` or `dirname(__DIR__, 2) . '/x.php'`; any other string that looks like a path counts as a path literal, as in JavaScript.

A name becomes a file through composer's autoloading, read from every `composer.json` in the tree, so path repositories count: the longest `psr-4` or `psr-0` prefix under `autoload` or `autoload-dev` whose file exists, else any file that declares the class or function, which covers classmaps and `files`. A name under one of the repository's own prefixes that no file declares is kept as the path autoloading would load, so deleting that file still reaches the files that named it. Anything else is a vendor or built-in name and is left out.

PHP tests aren't matched by default. Name them, and the runner that runs them:

```toml
[tests]
match = ["tests/**/*Test.php"]

[[tests.runners]]
id = "phpunit"
match = ["tests/**/*Test.php"]
command = ["vendor/bin/phpunit", "{files}"]
```

In a Laravel application every test boots the app, and booting it loads `bootstrap/app.php`, which names every route file, which names every controller, so a change to one controller reaches every test. That's the safe answer, and the one this gives today. Views, routes, bindings and the other links a framework makes at runtime will come from framework packs, which let a feature test reach the controller behind the route it calls without the walk passing through the app's boot.

## Go

`.go` files are scanned as the `go` provider. A Go package is every `.go` file in one directory, so:

- a file that imports a package depends on each of its non-test files;
- a test file depends on every other `.go` file in its directory, the package's own and its other test files, since they're compiled together;
- a `//go:embed` pattern depends on the file it names, every file under a directory it names, or the files a glob matches.

Import paths resolve through every `go.mod` in the tree, so a `go.work` workspace's modules are all found. The module whose path is the longest prefix of the import owns it, and a `replace` with a local path (`=> ../lib`) points a module at that directory. An import no module in the tree owns is looked for under the importing module's `vendor/`, and otherwise is the standard library or a dependency, left out. Build constraints aren't read, so a file for another platform still counts, which only ever adds tests.

`go test` takes packages rather than files, so its runner uses `{packages}`, which expands to one `./dir` per directory holding a selected test, or `./...` when everything runs:

```toml
[tests]
match = ["**/*_test.go"]

[[tests.runners]]
id = "go"
match = ["**/*_test.go"]
command = ["go", "test", "{packages}"]
```

Each Go file depends on its module's `go.mod` and `go.sum` (edge kind `manifest`), since they pick the versions of everything it imports. A dependency bump in one module therefore selects that module's tests and the tests of every module importing it, not the whole repository. A change to `go.work` or `go.work.sum`, which spans the workspace, runs everything by default (`plan.run_all`), as a lockfile does. To have a `go.mod` change run everything anyway, add `**/go.mod` to `plan.run_all`.

## Python

`.py` files are scanned as the `python` provider, with tree-sitter, so an import inside a function or a `try` counts like one at the top. A file depends on:

- the module each `import a.b` names, and the package `__init__.py` files above it, which importing it runs;
- for `from a.b import c`, the module `a.b.c` when there is one, else `a.b`, whose name `c` is; `from a.b import *` names `a.b`;
- relative imports (`from . import x`, `from ..models import y`) from the file's own package;
- any string that looks like a path, as in JavaScript.

Top-level modules are looked for under the repository root, each folder holding a `pyproject.toml`, `setup.py` or `setup.cfg`, and the `src` folder beside any of those, the ones above the importing file first. A module found under none of them is the standard library or an installed package, and is left out.

pytest loads every `conftest.py` from the test's folder up to the root before the test, so a test file (`test_*.py` or `*_test.py`) and a `conftest.py` depend on each `conftest.py` above them. A `conftest.py` that imports much of the package makes most changes reach every test, which is what pytest does too.

```toml
[tests]
match = ["tests/**/test_*.py"]

[[tests.runners]]
id = "pytest"
match = ["tests/**/test_*.py"]
command = ["pytest", "{files}"]
```

A change to `pyproject.toml`, `setup.py`, `setup.cfg`, `requirements*.txt`, `poetry.lock`, `uv.lock` or `Pipfile.lock` runs everything by default (`plan.run_all`).

## Other languages: external providers

The built-in scanners read JavaScript, TypeScript, PHP, Go and Python. For any other language, or a build tool that already knows its own graph, a `[[graph.providers]]` entry names a command that prints the graph for the files it claims:

```toml
[[graph.providers]]
id = "go"
command = ["./tools/go-graph.sh"]
files = ["**/*.go"]
```

- **The command** runs at the project root and reads the files it claims on stdin, one per line. It prints `{"version": 1, "edges": [{"from": "pkg/a_test.go", "to": "pkg/a.go"}]}`: repo-relative paths, where `from` depends on `to`. The shape is a public contract, committed as [`provider-v1.schema.json`](provider-v1.schema.json).
- **Claims:** a file a provider's `files` match belongs to that provider, and the built-in scanner leaves it alone. When two providers match one file, the one whose pattern has the longer literal part before its first wildcard wins (`src/go/**` over `**/*.go`), the earlier one on a tie, and `graph stats` reports the conflict.
- **Its edges** count only from files it claims, to files in the tree; the rest are counted as ignored in `graph stats`. They're followed like imports, with their own edge kind, `provider`.
- **A provider that fails** (it can't start, exits non-zero, runs past `timeout_seconds`, 120 by default, or prints something that isn't a version 1 graph) tells nothing about its files. A change to one of them runs every test, with a `provider-failed` warning naming the provider and why, as other uncertainty does.
- **`graph stats`** lists each provider with its files and edges, the built-in scanners as `typescript`, `php`, `go` and `python`.

Build tools that already know their graph plug in the same way, through a command that turns their output into this shape, such as `cargo metadata` for Rust, or a monorepo tool's project graph.

## Coverage maps

Some dependencies only show up when the code runs: a container that builds a class from a string, `importlib.import_module`, framework wiring. A coverage run sees them all. A coverage map records which source files each test ran, and its edges (kind `coverage`) join the static graph. A change reaches a test through either, and a new file the map predates still reaches its tests through its imports. It's off by default, since it needs a full coverage run to make.

Make the map from a coverage run's report, in the same job:

```bash
# PHPUnit, with pcov or Xdebug
vendor/bin/phpunit --coverage-xml build/coverage-xml
fairlead coverage import --format phpunit-xml build/coverage-xml

# pytest
pytest --cov=src --cov-context=test
coverage json --show-contexts -o coverage.json
fairlead coverage import --format coverage-py coverage.json
```

and name it:

```toml
[graph.coverage]
map = ".fairlead/coverage.json"
max_age_days = 14
```

- **PHPUnit** names each test `Class::method`, so the test file is the file that declares the class, through the same autoloading as [PHP](#php). **coverage.py** needs the per-test contexts pytest-cov records with `--cov-context=test`; lines run outside any test, such as imports at collection, count for none.
- **The map** is `{"version": 1, "commit": ..., "created": ..., "source": ..., "tests": {"tests/a_test.py": ["src/a.py", ...]}}`, stamped with the commit it tested and the day it was made. The shape is a public contract, committed as [`coverage-v1.schema.json`](coverage-v1.schema.json), so any tool can write one. A path from another checkout is matched by its longest tail that's a file here.
- **A map older than `max_age_days`** leaves a `coverage-stale` warning on the plan, naming its date and commit; **one that can't be read** leaves `coverage-unreadable`, and the plan uses the static graph alone. Neither fails the plan. A change to the map itself selects nothing.
- **`graph stats`** reports the map's edges, tests, source, date and commit, and how many of its pairs name a file no longer in the tree.

Refresh it on a schedule, such as a nightly workflow that runs the suite with coverage, imports the report and commits the map or keeps it where CI can fetch it.

## Barriers

In a typical server, every area imports a shared module for its routes or its auth, and that module imports every area. A walk through it reaches every test from any change. A barrier stops that:

```toml
[graph]
barrier = ["apps/api/src/{http,db,auth}/**"]
```

The walk reaches a barrier file, but doesn't go on to the files that import it. That includes a changed barrier file, which reaches nothing and is left to `tests.unreached`, unless `plan.run_all` or an owner rule covers it. So list a barrier's paths in `plan.run_all` when a change to them should run everything. `graph why` still shows a chain through a barrier and says the plan stops there, and `test --explain` names the barrier that kept a test out.

A rule edge is a claim the imports can't check. Replay measures it: a failure in a test the rule doesn't reach shows up as a miss.

## Resolution, without an install

Specifiers resolve the way Node and TypeScript do: tsconfig `paths` and project references (the nearest tsconfig to each file), `.js` specifiers that point at `.ts` files, directory `index` files, and package `exports` with the conditions in `graph.conditions`.

Workspace packages, from `workspaces` in the root `package.json` or `packages` in `pnpm-workspace.yaml`, resolve without `node_modules`: Fairlead shows the resolver a virtual `node_modules` that points at each package's folder. So a plan can run before `npm install`, and in CI before the install step.

- An import that lands in a package's build output (its tsconfig `outDir`, not in git) maps back to the same path under `rootDir`, whether or not a local build has put the output on disk.
- A workspace import that still lands on a missing or git-ignored file becomes an edge to every file of that package, which is always safe.
- A tsconfig that `extends` something that can't be read (a shared config published as a package, before install) is skipped for that file, and counted in `graph stats`.

Files larger than 256 KB, almost always generated, are scanned for import strings instead of parsed. The scan can't tell a comment from code, so it takes `from`, `import` and `require` only where they start a word, and only a specifier without whitespace, which no module name has: a doc comment reading `from "My booking"` isn't an import.

## Speed

A cold build of a 2,000-file workspace takes well under the 1.5 second budget on 4 cores, and a warm one under half a second. CI checks both on every pull request.

Parsing is most of a cold build. Fairlead keeps each file's parse result in `.git/fairlead/parse-cache.json`, keyed by the file's git blob id (the same id `git hash-object` prints) and its extension, so a file is parsed again only when its bytes change. The cache sits inside the git directory, so it's never committed and needs no `.gitignore` line; in a worktree it goes in that worktree's git directory. A new Fairlead version starts it over, entries for files that are gone drop out on the next build, and an unreadable cache is rebuilt. `graph stats` reports hits and files parsed. To turn it off, set `graph.cache = false`, or `FAIRLEAD_GRAPH__CACHE=false` for one run.

On the benchmark repositories, 4 cores:

| Repository | Cold | Warm |
| --- | --- | --- |
| Effect-TS/effect | 1.19 s | 0.07 s |
| pnpm/pnpm | 1.02 s | 0.26 s |
| vitest-dev/vitest | 0.41 s | 0.05 s |
