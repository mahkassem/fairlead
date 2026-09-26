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

An `import()` or `require()` whose argument isn't a plain string is counted as an unknown dynamic import; the planner treats the file as depending on its whole module.

## Edges the imports don't show

Some tests reach their code without importing it. An API test boots the server and calls it over HTTP, so no import joins the test to the area it exercises. A rule adds that edge, and the planner follows it like an import:

```toml
[[graph.edges]]
from = "apps/api/test/api/{area}{,-*}.test.ts"
to = ["apps/api/src/{area}/**"]
```

Each file matching `from` depends on every file its `to` globs match. A `{name}` stands for one path segment, with the same value on both sides, so `orders.test.ts` and `orders-refunds.test.ts` depend on `src/orders/`. Its value is read from the side where it's a whole segment: `{area}*` alone would also take `orders-refunds` as the name. `config check` refuses a `{name}` that is never a whole segment, or a `to` glob that names different ones than `from` (a `to` with none links to the same files for every match). `graph stats` counts rule edges, and names any rule that linked no file.

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

Files larger than 256 KB, almost always generated, are scanned for import strings instead of parsed.

## Speed

A cold build of a 2,000-file workspace takes well under the 1.5 second budget on 4 cores, and a warm one under half a second. CI checks both on every pull request.

Parsing is most of a cold build. Fairlead keeps each file's parse result in `.git/fairlead/parse-cache.json`, keyed by the file's git blob id (the same id `git hash-object` prints) and its extension, so a file is parsed again only when its bytes change. The cache sits inside the git directory, so it's never committed and needs no `.gitignore` line; in a worktree it goes in that worktree's git directory. A new Fairlead version starts it over, entries for files that are gone drop out on the next build, and an unreadable cache is rebuilt. `graph stats` reports hits and files parsed. To turn it off, set `graph.cache = false`, or `FAIRLEAD_GRAPH__CACHE=false` for one run.

On the benchmark repositories, 4 cores:

| Repository | Cold | Warm |
| --- | --- | --- |
| Effect-TS/effect | 1.19 s | 0.07 s |
| pnpm/pnpm | 1.02 s | 0.26 s |
| vitest-dev/vitest | 0.41 s | 0.05 s |
