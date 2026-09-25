# Introduction

Fairlead keeps a coding agent on course in your repository. It enforces your
project's rules at the moment the agent acts, gives it only the context that
applies to the files in front of it, runs only the tests a change can reach,
keeps project memory small, and measures whether all of that is working.

It is a single Rust binary with no project-specific logic: everything about
your repository lives in your own `fairlead.toml`.

This book grows with each milestone. Today it covers installing the
bootstrap release.
