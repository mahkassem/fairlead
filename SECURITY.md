# Security

Fairlead runs inside agent hooks and CI, so a vulnerability matters.

**Report privately** through
[GitHub security advisories](https://github.com/mahkassem/fairlead/security/advisories/new).
Please do not open a public issue. You'll get an answer within a few days.

**What protects a release**

- Release artifacts carry checksums and GitHub build attestations, and the
  installer verifies the checksum.
- The Composer package is a launcher that downloads the release's archive
  from github.com over HTTPS, redirects included, and checks it against the
  release's `.sha256` file before unpacking it. Its trust root is the shell
  installer's: GitHub's TLS and the tag ruleset below. The checksum is served
  beside the archive, so it proves the download is intact, not who built it;
  `FAIRLEAD_VERIFY_ATTESTATION=1` adds `gh attestation verify`, which checks
  that the release workflow built it and installs nothing when it fails. A
  binary in the system temp directory is run only from a folder the user owns
  and nobody else can write to.
- Publishing to npm uses npm trusted publishing: npm accepts a publish only
  from `release.yml` (which calls `publish-npm.yml`) in the `release`
  environment, so there is no npm token to leak, and each version carries a
  provenance attestation.
- Publishing to crates.io uses crates.io trusted publishing the same way:
  each crate accepts a publish only from `release.yml` (which calls
  `publish-crates.yml`) in the `release` environment, with a token that
  lives for that job and is revoked when it ends.
- The Homebrew formula is pushed to the `mahkassem/homebrew-tap` repository
  by `publish-homebrew.yml`, also in the `release` environment, with a
  fine-grained token that can write to that one repository and is stored
  only as a secret of that environment. The formula carries each build's
  checksum, which Homebrew checks on install.
- The `release` environment requires a maintainer's approval before any of
  these publishes. The GitHub Release is created before the approval, so the
  tag is its gate: rulesets let only admins create `v*` tags and nobody move
  or delete one.
- CI checks every workflow with zizmor, keeps `release.yml` identical to what
  `dist` generates, audits dependencies with `cargo deny`, and scans for
  secrets with GitGuardian. Dependabot keeps actions and crates current.

**Using Fairlead safely**

- Pin the GitHub Action by commit SHA, and set its `version` input to a
  release tag: the default, `latest`, follows new releases.
- Fairlead never sends data anywhere unless a project configures it to.
