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

## Check it

```sh
fairlead --version
fairlead doctor
```

`doctor` prints the version, the platform and the config file Fairlead would
use from the current directory.

## A first config: `fairlead init`

```sh
fairlead init             # write fairlead.toml from what the repository shows
fairlead init --dry-run   # print it instead
fairlead init --force     # replace a fairlead.toml that's already there
```

`init` reads the repository's manifests, lockfiles and test files, and writes
a `fairlead.toml` with a `[[tests.runners]]` entry for each runner it finds,
each with a comment saying where it came from. A runner goes in only when its
`match` finds a test file.

| It finds | It writes |
|---|---|
| `vitest`, `jest` or `mocha` in `package.json`, or `bun test` or `node --test` in the test scripts, one `npm run` step deep | that runner, through the lockfile's package runner (`bun x`, `pnpm exec`, `yarn`, `npx`) |
| `go.mod` at the root | `go test {packages}` on `**/*_test.go` |
| `pyproject.toml`, `setup.py`, `setup.cfg`, `pytest.ini` or `tox.ini` | pytest on `**/test_*.py` and `**/*_test.py`, through `uv run` or `poetry run` when their lockfile is there |
| `composer.json` | `php artisan test`, Pest or PHPUnit on `tests/**/*Test.php` |
| `Cargo.toml`, `gradlew`, `pom.xml`, a `.csproj`, or a `Gemfile` with `spec/` | a `[[checks]]` entry that runs the whole suite on any change in that language, which the import graph doesn't read yet |

It then loads and validates what it wrote, and prints the test files each
runner matched. What it can't tell, such as a runner's options in
`scripts.test` or a Go module below the root, it prints as a note. It never
replaces a config without `--force`.
