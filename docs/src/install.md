# Install

## Shell (macOS and Linux)

```sh
curl -fsSL https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.sh | sh
```

## PowerShell (Windows)

```powershell
powershell -c "irm https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.ps1 | iex"
```

## npm, bun or pnpm

```sh
bun add -d fairlead
```

The package pins the version in `package.json`, so everyone on the project
runs the same Fairlead.

The package downloads its binary when it installs. Bun runs no install
script it doesn't trust, so there the first `fairlead` command downloads it
instead; add `"trustedDependencies": ["fairlead"]` to `package.json` to have
`bun install` do it. *Since 0.12.0:* two first runs at once, such as parallel
git hook jobs, no longer race on that download: each extracts on its own
and the first to finish is used ([#211](https://github.com/mahkassem/fairlead/issues/211)).

*Since 0.13.0:* the package picks the build for the system's C library: the
musl build on Alpine and other musl systems, found from the `ldd` script,
then what Node reports, the musl loader in `/lib` and `ldd --version`. On a
platform with no build, the install prints a warning and succeeds, so a
project or container image that never runs Fairlead still installs; running
`fairlead` there exits 1 and prints the `cargo install` line that builds it
from source ([#218](https://github.com/mahkassem/fairlead/issues/218)).

## Platforms

Each release has a build for:

| System | Builds |
|---|---|
| Linux, glibc 2.35 or later | x86_64, aarch64 |
| Linux, musl (Alpine), or an older glibc | x86_64, aarch64, statically linked |
| macOS | x86_64, aarch64 |
| Windows | x86_64, which also runs on ARM64 Windows |

*Since 0.13.0:* the musl builds. The shell installer and the npm package
choose one when `ldd --version` or Node says the C library is musl; the npm
package also chooses one when glibc is older than the glibc build needs.
Anywhere else, build from source with Rust 1.95 or later, naming the
release you want:

```sh
cargo install --locked --git https://github.com/mahkassem/fairlead --tag vX.Y.Z fairlead
```

## Check it

```sh
fairlead --version
fairlead doctor
```

`doctor` prints the version, the platform and the config file Fairlead would
use from the current directory.
*Since 0.13.0:* it also prints the binary's path and the versions the repository names: the spec in
`package.json`, the package installed under `node_modules` and the config's `fairlead` floor. It warns
when `package.json` asks for a later version than the binary, as when a new worktree's
`node_modules` is a copy of an older one, and when the installed package isn't the binary that
runs, as when a global install comes first on the PATH.
`doctor --score` rates how ready the repository is, out of 100, and names the
fixes worth the most ([Readiness score](score.md)).

## A first config: `fairlead init`

```sh
fairlead init             # write fairlead.toml from what the repository shows
fairlead init --dry-run   # print it instead
fairlead init --force     # replace a fairlead.toml that's already there
fairlead init --pack next # also extend a framework pack (since 0.11.0)
```

`init` reads the repository's manifests, lockfiles and test files, and writes
a `fairlead.toml` with a `[[tests.runners]]` entry for each runner it finds,
each with a comment saying where it came from. A runner goes in only when its
`match` finds a test file.

| It finds | It writes |
|---|---|
| `vitest`, `jest`, `ava` or `mocha` in `package.json`, or `bun test` or `node --test` in the test scripts, one `npm run` step deep | that runner, through the lockfile's package runner (`bun x`, `pnpm exec`, `yarn`, `npx`), on the standard test-file names, `ava.files` for AVA, and any other file that imports the runner (`from "vitest"`), such as a `test.ts` beside each function |
| *Unreleased:* Bun 1.4.4 or later in `package.json`'s `packageManager` (`bun@1.4.4`) or in `.bun-version` | `bun test --parallel --no-isolate {files}` for the `bun test` runner, and a `bun-fmt` check, `bun fmt --check {files}` on the changed JavaScript and TypeScript files, commented out unless a script already runs `bun fmt` |
| `go.mod` at the root | `go test {packages}` on `**/*_test.go` |
| `pyproject.toml`, `setup.py`, `setup.cfg`, `pytest.ini` or `tox.ini` | pytest on `**/test_*.py` and `**/*_test.py`, through `uv run` or `poetry run` when their lockfile is there |
| `composer.json` | `php artisan test`, Pest or PHPUnit on the test directories `phpunit.xml` or `phpunit.xml.dist` names, else `tests/**/*Test.php` |
| `Cargo.toml`, `gradlew`, `pom.xml`, a `.csproj`, or a `Gemfile` with `spec/` (RSpec) or `test/*_test.rb` (minitest) | a `[[checks]]` entry that runs the whole suite on any change in that language, which the import graph doesn't read yet |

*Unreleased:* on Bun 1.4.4 or later, the `bun test` runner runs its files
in parallel workers. `--no-isolate` lets each worker run several files in one
process: isolated `--parallel`, a process per file, can be slower than serial
on suites of many small files (8.5 s against 3.1 s on one we measured), while
`--parallel --no-isolate` was the fastest on every suite measured. A parallel
run can turn up a test that depends on the order files run in. The gain
differs by repository, and `fairlead test bench` measures it on yours: serial,
`--parallel` and `--parallel --no-isolate`, each run twice. A version init
can't read, such as `bun@latest` or a range, or none at all, keeps
`bun test {files}`.

The `bun-fmt` check gates formatting per change: only the files a change
touches must be formatted, so a repository adopts the formatter a file at a
time instead of in one reformat that touches most of its files. Init writes it
commented out, since turning it on is the team's decision, and live when a
script in `package.json` already runs `bun fmt`. A plan that runs everything
checks every file it matches, as any check's `{files}` does
([Planning](plan.md)).

`[tests] match` is written as `{ replace = [...] }`, the runners' patterns and
nothing else, so a test file no runner here covers (a JavaScript test in a
Python project, say) stays out of the plan instead of stopping it.
It then loads and validates what it wrote, and prints the test files each
runner matched. What it can't tell, such as a runner's options in
`scripts.test` or a Go module below the root, it prints as a note. It never
replaces a config without `--force`. When the manifests show a framework
with a [pack](graph.md#framework-packs), Laravel or Next.js, it names the pack
and writes it only with `--pack`.
