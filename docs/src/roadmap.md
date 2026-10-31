# Roadmap

| Milestone | Delivers | Status |
|---|---|---|
| K0 | Workspace, CI, release pipeline, installers, docs | Shipped in 0.1.0 |
| K1 | Config and the test plan (import graph, test classes, CI plan), replay and the benchmarks | Shipped in 0.2.0 |
| K2 | Guard rules, Claude Code hooks, git hooks, event log | Shipped in 0.4.0 |
| K3 | Receipts, workflows, done gates, the change brief ([#45](https://github.com/mahkassem/fairlead/issues/45)), CI report, Codex hooks | Shipped in 0.6.0 and 0.7.0: the brief, the done gate and Stop hook, the receipt, the CI report and escapes, and Codex and Gemini CLI hooks |
| K4 | Memory, search, skills and skill routing ([#43](https://github.com/mahkassem/fairlead/issues/43)), import | Shipped in 0.8.0. Left: `skills init` proposing a mapping, which waits on real hit-rate data from `skills report` |
| K5 | MCP server, trackers, headless runner, token sources | Shipped in 0.10.0 and 0.11.0: the MCP server, trackers, token sources and escapes filed from CI. The headless runner waits on a decision about spend, since every run costs API money |
| K6 | Blueprints, readiness score, insights, 1.0 | Shipped in 0.10.0 and 0.11.0: the PHP scanner, blueprints, `doctor --score`, `insights --suggest` and the weekly summary. Left for 1.0: a Laravel service onboarded with its score before and after, a suggested config change accepted, crates.io and a Homebrew tap, and the tag |

Alongside the milestones, [any stack, any framework, any layout](https://github.com/mahkassem/fairlead/issues/47) brings more of what Fairlead reads. Python, Go, PHP, Vue, Svelte and Astro components, external graph providers and coverage maps shipped in 0.5.0, and Java and Kotlin in 0.7.0; framework packs (Laravel's in 0.10.0, Next.js's in 0.11.0) and folders of several repositories followed in 0.10.0 and 0.11.0. *Unreleased:* named imports are followed through re-exports to the file that declares each name ([#221](https://github.com/mahkassem/fairlead/issues/221)).

Each milestone is an issue on GitHub with its exit criteria.
