# Security

Fairlead runs inside agent hooks and CI, so a vulnerability matters.

**Report privately** through
[GitHub security advisories](https://github.com/mahkassem/fairlead/security/advisories/new).
Please do not open a public issue. You'll get an answer within a few days.

**What we do to keep releases safe**

- Release artifacts carry checksums and GitHub build attestations, and the
  installer verifies the checksum.
- Only maintainers can push the version tags that start a release.
- Pin the GitHub Action by commit SHA in production workflows.
- Fairlead never sends data anywhere unless a project configures it to.
