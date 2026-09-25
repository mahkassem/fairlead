# Working on Fairlead (for agents and people)

Fairlead is a standalone Rust CLI that makes any repository agent friendly.

- **Never add project-specific logic.** No company, product or repository
  names, and no logic modelled on one adopter, in code, docs, fixtures,
  issues, pull requests, comments or commit messages. Design from general
  practice and benchmark on public repositories. `python3 scripts/leak_check.py`
  fails CI on the hashed denylist in `.github/leak-denylist.sha256`, and the
  `leak-text` workflow checks public text after it is posted; check a draft
  first with `python3 scripts/leak_check.py --text < draft.md`.
- **Checks before a push:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, `python3 scripts/leak_check.py`, and `zizmor --offline .` when
  a workflow changed.
- **Small on purpose:** a file stays under 1,000 lines and a function under
  120; the release binary stays at 15 MB or less (CI checks it).
- **Work is tracked in GitHub Issues**, one per milestone (K0 to K6). Branches
  are `feat/`, `fix/` or `chore/`; every change lands through a pull request.
- **Say what you could not prove** in the pull request.
