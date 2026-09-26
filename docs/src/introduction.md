# Introduction

<img class="brand-logo light" src="images/fairlead-logo.svg" alt="Fairlead">
<img class="brand-logo dark" src="images/fairlead-logo-dark.svg" alt="Fairlead">

Fairlead keeps a coding agent on course in your repository. It enforces your
project's rules at the moment the agent acts, gives it only the context that
applies to the files in front of it, runs only the tests a change can reach,
keeps project memory small, and measures whether all of that is working.

It is a single Rust binary with no project-specific logic: everything about
your repository lives in your own `fairlead.toml`.

This book grows with each milestone. Today it covers installing Fairlead, its
configuration, the import graph it builds without an install, the test plan
and how to run it in CI, and replay, which measures the plan against real CI
failures, with the [benchmarks](benchmarks.md) that come from it. The
[roadmap](roadmap.md) says what comes next.
