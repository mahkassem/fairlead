# Trackers

*Since 0.10.0*, `[tracker]` links a change to the task it serves. When the branch name carries a task id, the brief shows that task at its top: its title, status, link and, where the tracker gives one, what "done" means for it.

```toml
[tracker]
kind = "command"                 # none, command, github or agent
id = "T[0-9]+"                   # every task id must match this whole
branch = "(T[0-9]+)"             # finds the id in a branch name, such as feat/T1890-bump
get = ["bun", "run", "tasks", "--", "show", "--json"]
timeout = 20                     # seconds
```

| Key | Default | What it does |
|---|---|---|
| `kind` | `none` | Where tasks live (below). |
| `id` | `[0-9]+` for `github`, none otherwise | A regex every task id must match whole before Fairlead uses it. Required for `command` and `agent`. |
| `branch` | none | A regex that finds the id in the branch name: its first capture group, else the whole match. |
| `get` | `[]` | For `command`: the argv that prints one task as JSON. The id is appended as its last argument. |
| `timeout` | `20` | How long one tracker call may take, in seconds (1 to 120). |

## Kinds

- **`command`:** the team's own command. Fairlead runs `get` with the id as its last argument and reads one JSON object from its output: `id`, `title`, and optionally `url`, `status` and `done_when`. The command runs without a shell, inherits the environment as runners do, and is killed at the timeout. Only its first 64 KB of output is read.
- **`github`:** issues in the repository that `origin` points at (or `GITHUB_REPOSITORY`), read with `GITHUB_TOKEN` or `GH_TOKEN` when set. The id is the issue number.
- **`agent`:** Fairlead calls nothing. The brief names the task and tells the agent to look it up with its own tracker tool. This is for a team whose tracker is only an MCP tool the agent already has.

## Tracker text is data

A task's title and fields come from outside the repository, so before the brief shows them:

- control characters, terminal escape sequences, text-direction marks and zero-width characters are removed, and line breaks become spaces;
- the title is cut at 200 characters, the status at 40, and `done_when` at 1,000;
- only `https://` links are kept;
- the brief quotes the title and labels `done_when` as coming from the tracker.

A tracker that answers for a different id than the one asked for is refused.

An id reaches a command only after it matches `id` whole, and never when it starts with `-`. So a branch named `task/--delete-all` names no task, whatever the pattern allows.

## When it runs

The task is read once per brief, when the brief is first made, and kept with it, so a slow tracker costs one call per change. A tracker that fails, hangs or prints something else leaves a `tracker:` warning in the brief, and the brief is made without the task.
