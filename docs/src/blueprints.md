# Blueprints

*Since 0.10.0*, a blueprint is a named starting point for a kind of project. `fairlead init --blueprint NAME` writes one into the repository once: config on top of what `init` detects, starter files such as a SKILL.md, and the CI workflow. From then on, the project owns those files.

A [framework pack](graph.md#framework-packs) is different: a pack is a layer every load reads, and it changes with the release. A blueprint is written once, and it can name a pack in `extends`.

```sh
fairlead init --blueprint laravel-api
fairlead init --blueprint ./blueprints/ours         # a folder holding blueprint.toml
fairlead init --blueprint vite-react --dry-run      # print the config and the files it would write
```

Without `--blueprint`, `init` writes only what it detects. When the repository looks like a built-in blueprint, it says which one and the command that adds it.

## Built in

| Blueprint | Detected by | Adds |
|---|---|---|
| `laravel-api` | `artisan` | `extends = ["laravel"]`; migrations that never change once run; size limits on `app/**/*.php`; a SKILL.md for the HTTP layer, routed to `app/Http/**` and `routes/**`; `[stages]` and the CI workflow |
| `vite-react` | `vite.config.{ts,js,mts,mjs}` | size limits on `src/**/*.{ts,tsx}`; a SKILL.md for components, routed to `src/**/*.tsx`; `[stages]` and the CI workflow |

The SKILL.md files are a start, meant to be replaced with the team's own rules.

## A team's own

A blueprint is a TOML file: `--blueprint` takes its path, or a folder holding `blueprint.toml`. No blueprint is fetched over the network.

```toml
name = "payments-service"
description = "A payments service: the team's guard rules and its review skill"
detect = ["artisan"]          # files that suggest it; any one is enough
ci_workflow = true            # also write .github/workflows/fairlead.yml
config = '''
[guard.migrations]
files = ["database/migrations/*.php"]

[[skills.routes]]
skill = ".claude/skills/payments/SKILL.md"
paths = ["app/Payments/**"]
'''

[[files]]
path = ".claude/skills/payments/SKILL.md"
text = '''
---
name: payments
description: The rules for payment code.
---
...
'''
```

- **Placement:** the blueprint's top-level keys, such as `extends`, go at the top of `fairlead.toml`, and its tables after what `init` detected.
- **Checked first:** the whole config is checked before `fairlead.toml` is written, so a blueprint whose config doesn't load leaves nothing behind.
- **Files:** a file that's already there is kept unless `--force` is given. A file path must stay inside the repository.
