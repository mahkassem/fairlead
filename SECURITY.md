# Security

Fairlead runs inside agent hooks and CI, so a vulnerability matters.

**Report privately** through
[GitHub security advisories](https://github.com/mahkassem/fairlead/security/advisories/new).
Please do not open a public issue. You'll get an answer within a few days.

**What protects a release**

- Release artifacts carry checksums and GitHub build attestations, and the
  installer verifies the checksum.
- Publishing to npm uses npm trusted publishing: npm accepts a publish only
  from `publish-npm.yml` running in the `release` environment, so there is no
  npm token to leak, and each version carries a provenance attestation. That
  environment requires a maintainer's approval. The GitHub Release is created
  before the approval, so the tag is its gate: rulesets let only admins create
  `v*` tags and nobody move or delete one.
- CI checks every workflow with zizmor, keeps `release.yml` identical to what
  `dist` generates, audits dependencies with `cargo deny`, and scans for
  secrets with GitGuardian. Dependabot keeps actions and crates current.

**Using Fairlead safely**

- Pin the GitHub Action by commit SHA, and set its `version` input to a
  release tag: the default, `latest`, follows new releases.
- Fairlead never sends data anywhere unless a project configures it to.
