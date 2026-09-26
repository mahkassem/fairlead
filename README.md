<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/mahkassem/fairlead/main/assets/brand/fairlead-reversed-horizontal-dark.svg">
    <img alt="Fairlead" src="https://raw.githubusercontent.com/mahkassem/fairlead/main/assets/brand/fairlead-primary-horizontal-light.svg" width="360">
  </picture>
</p>

<p align="center"><strong>Guardrails and a guided path for coding agents.</strong></p>

<p align="center">
  <a href="https://github.com/mahkassem/fairlead/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/mahkassem/fairlead/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/mahkassem/fairlead/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/mahkassem/fairlead?color=008F67"></a>
  <a href="https://mahkassem.github.io/fairlead/"><img alt="Docs" src="https://img.shields.io/badge/docs-book-0B1618"></a>
</p>

A fairlead is the fitting on a boat that keeps a line running true, so it
doesn't chafe, tangle or pull off course. Fairlead does that for an agent
working in your repository. It is one Rust binary with no project-specific
logic: everything about your repository lives in your own `fairlead.toml`.

## What it does today

- **Tests smart.** `fairlead plan` builds your repository's import graph
  without an install and selects the tests a change can reach, plus canaries,
  owner rules and checks. `fairlead test --explain` says why a test is in the
  plan or isn't.
- **Runs in CI.** `fairlead ci plan` writes the plan for a pull request and
  `fairlead ci run` runs it, from the CLI or through the GitHub Action.
- **Proves its recall.** `fairlead replay` re-plans real CI failures from a
  repository's history and reports how many the plan would have caught. The
  [benchmarks](https://mahkassem.github.io/fairlead/benchmarks.html) run it
  every week on Effect, pnpm and vitest.
- **One config, checked.** `fairlead config check`, `show --origin` and
  `schema`, with layers from built-in defaults to `--set`, and every error
  naming its file and key.

## Where it's going

- **Guards:** your project's rules run as hooks on every edit and command,
  so a wrong move is stopped with the right command instead of failing CI
  minutes later.
- **Guides:** the agent asks what applies to the files in front of it and
  what its next step is, instead of reading pages of instructions.
- **Remembers, within limits:** small per-module memory with caps and review
  dates. A lesson becomes a check or it expires.
- **Measures:** speed, rework, escaped defects, tokens and cost, as counts
  only. It never phones home.

Claude Code comes first, then Codex. The [roadmap](https://mahkassem.github.io/fairlead/roadmap.html)
has milestones K0 to K6, each an [issue](https://github.com/mahkassem/fairlead/issues)
with its exit criteria.

## Status

Pre-alpha. The latest release, v0.1.1, ships the config commands. The test
plan, the CI commands and action, and replay are on `main`, and ship in v0.2.0
once the benchmarks meet its recall bar.

## Install

```sh
# macOS and Linux
curl -fsSL https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.sh | sh

# Windows (PowerShell)
powershell -c "irm https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.ps1 | iex"

# In a JavaScript or TypeScript project, pinned in package.json
bun add -d fairlead   # or: npm i -D fairlead
```

Then `fairlead doctor` says which binary, platform and config it would use.

## Quick start

```sh
fairlead config check                        # validate fairlead.toml
fairlead plan --base main                    # the tests and checks this branch can affect
fairlead test --explain src/a.test.ts        # why that test is in the plan, or isn't
fairlead graph why src/a.test.ts src/util.ts # how one file depends on another
```

## Documentation

The book is at [mahkassem.github.io/fairlead](https://mahkassem.github.io/fairlead/):
install, configuration, the import graph, the test plan, plans in CI, replay
and the benchmarks. Its source is in `docs/`.

## Brand

The logo, its colors and how to use them are in [`assets/brand`](assets/brand).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any additional
terms or conditions.
