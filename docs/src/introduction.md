# Introduction

Fairlead keeps a coding agent on course in your repository. It enforces your
project's rules at the moment the agent acts, gives it only the context that
applies to the files in front of it, runs only the tests a change can reach,
keeps project memory small, and measures whether all of that is working.

It is a single Rust binary with no project-specific logic: everything about
your repository lives in your own `fairlead.toml`.

This book documents Fairlead 0.9.0, the latest release. The book for an
earlier release is its `docs/` folder at that release's tag, such as
[`v0.5.1`](https://github.com/mahkassem/fairlead/tree/v0.5.1/docs/src).

It covers installing Fairlead and its configuration; the import graph it
builds without an install, with coverage maps for what imports can't show; the
test plan and how to run it in CI, with the run's report and the escapes the
default branch finds; the guard rules and the hooks that run them as an agent
works; the change loop an agent follows: the [brief](brief.md) before an
edit, the [done gate](done.md) before it's finished, and the
[receipt](receipt.md) after; and replay, which measures the plan against real
CI failures, with the [benchmarks](benchmarks.md) that come from it. The
[roadmap](roadmap.md) says what comes next.
