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

## Why Fairlead

Join a good team and you don't start from zero. Someone tells you which module
never to touch without running the migration check first, which test lies on
Windows, and which "quick fix" broke production last spring. That knowledge is
why a new teammate is useful in week two instead of month six.

A coding agent gets none of it. It opens your repository cold every time,
reads the same code again, makes the mistake your team already paid for, and
finds out in CI that it was an old lesson nobody told it. Fairlead exists so an
agent works like a member of the team, not a stranger trying things.

- **Every repository has a memory.** The hard lessons from past work live with
  the code, per module, where the agent meets them before it acts. A lesson is
  kept short, reviewed on a date, and turned into a check when it can be, so
  the memory stays true instead of growing into noise.
  *Today:* owner rules record which tests guard which code, and a quarantined
  test applies only while the evidence holds and until its date.
- **Set up once, then work like a teammate.** The agent's first job is to learn
  the repository: its modules, test runners, rules, and the commands that prove
  a change is right. That goes into one checked config, so every session starts
  where the last one left off.
  *Today:* `fairlead config check` validates every layer, and `show --origin`
  says where each value came from.
- **The right tool, not the nearest one.** The agent asks what applies to the
  files in front of it and gets the rule, the command and the next step for
  exactly those files.
  *Today:* `fairlead plan` names the tests and checks a change can reach, and
  `test --explain` says why each one is in or out.
- **The right skills for the code in front of it.** A frontend change
  shouldn't come with database advice. Fairlead picks the skills that apply to
  what a change reaches, just as it picks tests, and checks the agent used them.
  *Coming in K4* ([#43](https://github.com/mahkassem/fairlead/issues/43)).
- **Measured, not guessed.** Good and bad are numbers: whether the plan would
  have caught real CI failures, rework, escaped defects, tokens and cost.
  *Today:* `fairlead replay` re-plans real failures from a repository's CI
  history, and the [benchmarks](https://mahkassem.github.io/fairlead/benchmarks.html)
  measure that recall every week.
- **Fast because it remembers.** No rereading the codebase to rediscover what
  was learned last week. The graph, the plan and the lessons are already there.
  *Today:* the import graph is built without installing dependencies and
  cached between runs.
- **It learns where it's blind.** When a change reaches no test, Fairlead says
  so and points at the rule that's missing, so the team's knowledge grows
  exactly where it was thin.
  *Today:* `fairlead plan` lists every file no test depends on, and replay
  suggests the owner rule or check path that would have caught a miss.
- **Stopped before the mistake, not after.** Your rules run as the agent
  works, catching a wrong move in seconds instead of minutes later in CI.
  *Coming in K2.*

## What it does today

The test plan (`plan`, `test --explain`), plans in CI (`ci plan`, `ci run` and
the GitHub Action), `replay`, the import graph (`graph`), and the config
commands. The [quick start](#quick-start) shows them, and the
[book](https://mahkassem.github.io/fairlead/) covers each one.

## Status

Pre-alpha. The latest release, v0.1.1, ships the config commands. The test
plan, the CI commands and action, and replay are on `main`, and ship in v0.2.0
once the benchmarks meet its recall bar ([#20](https://github.com/mahkassem/fairlead/issues/20)).
Claude Code comes first, then Codex. The [roadmap](https://mahkassem.github.io/fairlead/roadmap.html)
has milestones K0 to K6, each an [issue](https://github.com/mahkassem/fairlead/issues)
with its exit criteria.

## Install

```sh
# macOS and Linux
curl -fsSL https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.sh | sh

# Windows (PowerShell)
powershell -c "irm https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.ps1 | iex"

# In a JavaScript or TypeScript project, as a dev dependency
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
