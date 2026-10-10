# Workspaces

Many teams keep a folder of repositories side by side, one per service or app, rather than one monorepo. *Since 0.10.0*, a `fairlead.toml` in that folder makes it a workspace: `fairlead plan` there plans each repository against its own base, and every command inside a repository reads the workspace file as a layer under the repository's own.

```toml
# ~/work/fairlead.toml
[workspace]

[[workspace.repos]]
path = "api"
base = "origin/main"

[[workspace.repos]]
path = "web"

[[workspace.repos]]
path = "libs/shared"
name = "shared"

# Anything else is shared by every repository, under its own file.
[[tests.runners]]
id = "vitest"
match = ["**/*.test.ts"]
command = ["vitest", "run", "{files}"]
```

| Key | Default | What it does |
|---|---|---|
| `workspace.repos` | `[]` | The repositories, each a `path` from the workspace folder to a git repository inside it. Empty means every git repository one level down, in name order. |
| `workspace.repos.name` | the folder's name | What `--repo` and the plan call it. Two repositories with one name is an error. |
| `workspace.repos.base` | the repository's remote default branch | The branch or commit its changes are measured from, through its merge base, as `--base` would be. |

## Planning from the folder

`fairlead plan` in the workspace folder plans every repository it lists:

- Each repository's changes are measured against its own base and merge base, so a repository with nothing new is left out of the plan and named under `unchanged`.
- `--base` overrides every repository's base for one run, and `--repo NAME` plans only that one.
- `--files` takes paths from where the command runs; each goes to the repository holding it, and a repository given none is unchanged.
- `--json` prints `{"repos": [{"name", "path", "plan"}], "unchanged": [...]}`, where each `plan` is a repository's plan exactly as `fairlead plan --json` prints it inside the repository, and `path` is from the workspace folder.

`fairlead test --explain FILE` in the folder explains `FILE` in the repository that holds it; for a check, which has no path, name the repository with `--repo`. From inside a repository, `--repo` plans another repository of the same workspace.

Every other command works on one repository, so in the workspace folder it stops and names the repositories to run it in.

## Inside a repository

Commands inside a repository find its config as always, walking up no further than the repository root. Then, outside CI, they look further up for a workspace file that lists the repository, or that lists none and has it one level down. That file is a layer under the repository's project file (see [Layers](config.md#layers)): its lists come first, and the repository's file can add to them, replace them with `{ replace = [...] }`, or set any value over them. A pack the workspace file names in `extends` is looked up from each repository's root.

A repository's own file can't declare `[workspace]`. In CI, where the checkout is one repository, the workspace file is never read, so each repository's CI plans itself exactly as it did.

## What's next

Edges across repositories are the second phase of [#52](https://github.com/mahkassem/fairlead/issues/52): a path dependency resolving into a sibling repository, a contract one repository owns and another consumes, and owner rules in the workspace file that select one repository's tests for another's changes. Until then, each repository's plan sees only its own changes.
