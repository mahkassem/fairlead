# Contributing

Thanks for helping. A few rules keep the project small and trustworthy.

- **Open an issue first** for anything bigger than a fix, so the design is
  agreed before the code.
- **Before you push:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, and `scripts/leak-check.sh`. CI runs the same, plus `cargo deny`.
- **Tests:** anything with a branch, a loop or a rule gets a test. Integration
  tests drive the built binary (`tests/`).
- **No project-specific logic.** Fairlead must work in any repository.
  Anything specific to one project belongs in that project's `fairlead.toml`,
  never here. Fixtures are synthetic.
- **Dependencies:** say why in the PR. Every new crate must pass `cargo deny`.
- **Commits** follow Conventional Commits in the imperative mood.

By contributing you agree that your contribution is dual licensed under
MIT OR Apache-2.0, as described in the README.
