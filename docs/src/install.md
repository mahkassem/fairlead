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

*Since 0.18.0:* a download that fails on the network, stalls for 30 seconds or
gets a 408, 429 or 5xx answer is tried again after 1, 2 and 4 seconds, and
each retry prints a line. A body that ends short of its length counts as a
failure, so a cut connection is retried instead of leaving a broken archive.
A 404, such as a version with no release, isn't retried.

### A binary fetched another way

*Since 0.18.0:* where the runner can't or shouldn't download, such as CI with
a cache or a mirror, set `FAIRLEAD_BINARY` to the binary's path. The install
then downloads nothing, and every `fairlead` run through the package runs
that file with the same arguments:

```yaml
- uses: actions/cache@v4
  with:
    path: ${{ runner.temp }}/fairlead
    key: fairlead-${{ runner.os }}-0.21.0
- run: |
    mkdir -p "$RUNNER_TEMP/fairlead"
    test -x "$RUNNER_TEMP/fairlead/fairlead" || curl -fsSL --retry 3 \
      https://github.com/mahkassem/fairlead/releases/download/v0.21.0/fairlead-x86_64-unknown-linux-gnu.tar.xz \
      | tar xJ --strip-components 1 -C "$RUNNER_TEMP/fairlead"
- run: npm ci
  env:
    FAIRLEAD_BINARY: ${{ runner.temp }}/fairlead/fairlead
```

Set it on every later step that runs `fairlead` too, or once for the job.
The file need not exist when the package installs, since a later step may
put it there. When it still isn't there at run time, `fairlead` exits 1 and
says so; unset the variable to go back to the download.

## Homebrew (macOS and Linux)

```sh
brew install mahkassem/tap/fairlead
```

*Since 0.14.0:* the formula lives in the
[mahkassem/homebrew-tap](https://github.com/mahkassem/homebrew-tap) tap and
installs the release's own build, checked against the release's checksums.
`brew upgrade fairlead` moves to each new release once it is published.

## Cargo (crates.io)

```sh
cargo install --locked fairlead
```

*Since 0.14.0:* builds the release from crates.io with Rust 1.95 or later, on
any platform Rust builds for, including one with no prebuilt build below.
`--locked` uses the dependency versions the release was tested with. The
`fairlead-*` library crates beside it on crates.io are its parts, published
only so it builds; their APIs have no semver promise.

## Composer

*Since 0.14.0:* for a PHP or Laravel project, with no Node needed:

```sh
composer require --dev mahkassem/fairlead
vendor/bin/fairlead --version
```

In a Laravel app, `vendor/bin/fairlead init` then finds `artisan` and
`laravel/framework`, uses the Laravel pack, and runs your tests with Pest or
PHPUnit, whichever `composer.json` names.

The package is a small PHP launcher, not the binary. Its first run downloads
the release build for the platform from the repository's GitHub releases, the
archives the shell installer uses, checks it against the release's `.sha256`
file before unpacking it, and keeps it; later runs start the binary straight
away, passing the arguments, input, output and exit code through. It runs the
release Composer installed, so the lock file keeps everyone on the project on
the same Fairlead. It picks the build as the npm package does: the musl build
when `ldd --version` says musl, when glibc is older than 2.35, or when neither
shows. It needs PHP 8.1 or later with the curl extension or `allow_url_fopen`,
and `tar`, which Windows 10 and later ship (a zip is unpacked with PHP's zip
extension when it's loaded).

The binary is kept in the first of these it can write to:
`FAIRLEAD_CACHE_DIR` when it's set, and then only there; the package's own
folder, `vendor/mahkassem/fairlead/.cache`, so a cached `vendor/` carries it;
`fairlead/` under `XDG_CACHE_HOME` (`~/.cache`) or `%LOCALAPPDATA%`; and a
folder of the user's own in the system temp directory. Each release and
platform gets its own folder, which nothing writes to once it's there, and
two first runs at once each unpack on their own and use whichever finished
first. To build an image or a CI cache that runs offline, run it once while
the network is there:

```dockerfile
RUN composer install --no-interaction && vendor/bin/fairlead --version
```

A branch install, such as `dev-main`, has no release build, so the launcher
refuses it unless `FAIRLEAD_VERSION` names a release (`FAIRLEAD_VERSION=0.14.0`),
which also overrides the installed version. When the download fails or
doesn't match its checksum, it names the URL, prints the `cargo install` line
under [Platforms](#platforms) and exits 1. Packagist lists a tag as soon as
it's pushed, before its release has built; a run in that window says
`no build for vX.Y.Z yet`. A platform with no build gets the `cargo install`
line too.

To also check the build's GitHub attestation, set
`FAIRLEAD_VERIFY_ATTESTATION=1` with the [GitHub CLI](https://cli.github.com)
installed: the launcher runs
`gh attestation verify <archive> --repo mahkassem/fairlead` before unpacking,
and installs nothing when the check fails or `gh` is missing. The checksum
proves the download is intact, not who built it
([Security](https://github.com/mahkassem/fairlead/blob/main/SECURITY.md)).

Listing the package on Packagist is a one-time step for the repository's
owner. Until it's listed, `composer require` finds it only with the
repository named in `composer.json`:

```json
"repositories": [{ "type": "vcs", "url": "https://github.com/mahkassem/fairlead" }]
```

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

*Since 0.14.0:* or from crates.io, `cargo install --locked fairlead --version X.Y.Z`
([Cargo](#cargo-cratesio)).

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
| *Since 0.14.0:* Bun 1.4.4 or later in `package.json`'s `packageManager` (`bun@1.4.4`) or in `.bun-version` | `bun test --parallel --no-isolate {files}` for the `bun test` runner, and a `bun-fmt` check, `bun fmt --no-script --check {files}` on the changed JavaScript and TypeScript files, commented out unless a script already runs `bun fmt` |
| `go.mod` at the root | `go test {packages}` on `**/*_test.go` |
| `pyproject.toml`, `setup.py`, `setup.cfg`, `pytest.ini` or `tox.ini` | pytest on `**/test_*.py` and `**/*_test.py`, through `uv run` or `poetry run` when their lockfile is there |
| `composer.json` | `php artisan test`, Pest or PHPUnit on the test directories `phpunit.xml` or `phpunit.xml.dist` names, else `tests/**/*Test.php` |
| `Cargo.toml`, `gradlew`, `pom.xml`, a `.csproj`, or a `Gemfile` with `spec/` (RSpec) or `test/*_test.rb` (minitest) | a `[[checks]]` entry that runs the whole suite on any change in that language, which the import graph doesn't read yet |

*Since 0.14.0:* on Bun 1.4.4 or later, the `bun test` runner runs its files
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
replaces a config without `--force`.

*Since 0.19.1:* three things a first setup got wrong:

- **Playwright specs get a runner of their own.** A spec is named like a
  unit test, so the unit runner's patterns matched it and `bun test` or
  Vitest was handed files it can't run. Init finds the specs by their
  import of `@playwright/test`, writes a `playwright` runner for them and an
  `exclude` on the unit runner, and notes that the runner wants a
  [`smoke` set](done.md#the-budget).
- **A repository with no tests yet starts with the guard.** Where init
  finds no runner it writes [the guard-only setup](#the-guard-only), where
  it used to write nothing and exit 1. `fairlead doctor` says when the
  repository has tests a runner can run, and `fairlead init --dry-run`
  prints that runner.
- **The steps after init, in order:** `hooks install`, `hooks install --git`,
  `agents sync`, `ci workflow --write`, `done` once, then `doctor` for what
  is still missing.
 When the manifests show a framework
with a [pack](graph.md#framework-packs), Laravel or Next.js, it names the pack
and writes it only with `--pack`.

### The guard only

*Since 0.18.0:* for a stack the test planner doesn't read, such as Laravel,
Rails, Flutter or Swift, or a team that wants the guard before anything
else:

```sh
fairlead init --profile guard            # write fairlead.toml and add the git hook
fairlead init --profile guard --dry-run  # print the config instead
```

It writes `profile = "guard"` and no runners, with the
[secret-path rule](guard.md#secret-paths), the
[gitleaks preset](guard.md#gitleaks) and, when the tree holds Laravel, Rails,
Prisma or plain SQL migrations, a [`[guard.migrations]`](guard.md#migrations)
table for that folder, its base the remote's default branch. Numbered
migrations, as in Rails or `migrations/*.sql`, also get unique numbers
checked against the base fetched at a push. Then it adds the commit stage
to `lefthook.yml`, and the push stage when the config fetches, as
`hooks install --git` would. It says when gitleaks isn't installed.

In this profile `fairlead done` runs only `guard check` and, with
[`[pr]` rules](pr.md), `pr check`; `doctor` prints `profile: guard`; and
`doctor --score` leaves out the items that score the test plan
([Readiness score](score.md#the-guard-profile)). In CI, run `fairlead guard check`, as the
action's `command: guard check`, and `fairlead pr check` for the `[pr]` rules.
