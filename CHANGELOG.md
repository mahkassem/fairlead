# Changelog

## 0.1.1 (2026-09-26)

### New

- `fairlead config check`, `config show [--origin]` and `config schema`. One config file, `fairlead.toml` or `fairlead.yaml`, with layers: built-in defaults, the project file, a local file, `FAIRLEAD_*` environment variables and `--set`. Lists append across layers, and `{ replace = [...] }` replaces one. Every error names its file and key. See the [configuration docs](https://mahkassem.github.io/fairlead/config.html).
- `fairlead doctor` says whether the config is valid.

### Changed

- The npm package is published through npm trusted publishing, with provenance, from an approval-gated release environment. There's no npm token.
- The minimum Rust version for building from source is 1.89.

### Security

- CI checks workflows with zizmor, keeps the generated release workflow unchanged, and runs CodeQL.
- A hashed leak check keeps private names out of files, issues, pull requests, comments and commit messages.

## 0.1.0 (2026-09-25)

- First release: `fairlead --version` and `fairlead doctor`, installers for Linux, macOS and Windows, and the npm package.
