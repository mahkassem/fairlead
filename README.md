# Fairlead

Guardrails and a guided path for coding agents.

A fairlead is the fitting on a boat that keeps a line running true, so it
doesn't chafe, tangle or pull off course. Fairlead does that for an agent
working in your repository:

- **Guards:** your project's rules run as hooks on every edit and command,
  so a wrong move is stopped with the right command instead of failing CI
  minutes later.
- **Guides:** the agent asks what applies to the files in front of it and
  what its next step is, instead of reading pages of instructions.
- **Tests smart:** a change runs the tests it can reach, plus a canary set.
  Main and nightly still run everything.
- **Remembers, within limits:** small per-module memory with caps and review
  dates. A lesson becomes a check or it expires.
- **Measures:** speed, rework, escaped defects, tokens and cost, as counts
  only. It never phones home.

It is one Rust binary, works in any repository, and supports Claude Code
first and Codex next.

## Status

Pre-alpha. This release is the bootstrap: `fairlead --version` and
`fairlead doctor`. The roadmap is milestones K0 to K6 in the
[issues](https://github.com/mahkassem/fairlead/issues).

## Install

Once the first release is published:

```sh
# macOS and Linux
curl -fsSL https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.sh | sh

# Windows (PowerShell)
powershell -c "irm https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.ps1 | iex"

# In a JavaScript or TypeScript project
bun add -d fairlead   # or: npm i -D fairlead
```

## Documentation

The book lives in `docs/` and is published to GitHub Pages.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any additional
terms or conditions.
