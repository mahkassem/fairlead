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

An `import()` or `require()` whose argument isn't a plain string is counted as an unknown dynamic import; the planner treats the file as depending on its whole module.

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
