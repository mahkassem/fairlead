# MCP server

*Since 0.10.0*, `fairlead mcp` serves Fairlead's read-only commands to an agent as MCP tools, over stdio. An agent that speaks MCP can ask for the plan or a brief without a shell, and gets the same JSON the command prints with `--json`.

Register it with the agent, from the repository root:

```sh
claude mcp add fairlead -- fairlead mcp          # Claude Code
```

For another client, the server is the command `fairlead mcp` with no arguments, started in the repository. Where the project runs its own copy of Fairlead, use that command instead, such as `bun x fairlead mcp`.

## Tools

| Tool | Runs | Returns |
|---|---|---|
| `plan` | `plan --json` | the plan, as in [the plan's schema](plan-v1.schema.json) |
| `explain` | `test --explain` | text: why a test or check is in the plan, or why not |
| `brief` | `brief --json` | the brief for the paths a change will touch |
| `context` | `context --json` | the brief, plus what to read around the paths |
| `find` | `find --json` | hits from lessons, skills, headings and declared names |
| `receipt` | `receipt --json` | what the change did against its brief, and the done gate |
| `next` | `next` | text: the next step before the change is done |
| `lessons` | `lessons list --json` | the lessons, with scope and review dates |
| `skills_report` | `skills report --json` | skill routing's hit rate |

Every tool is read-only. `done` isn't one of them: it runs the tests, which takes minutes, and an MCP call can't be stopped midway. Run `fairlead done` through the agent's shell, where the hooks see it.

Paths are from the repository root and must stay inside it. Branch names, sessions and other values can't start with `-`, so none can pass for an option. A refused or failed call comes back as a tool result with `isError`, with the reason as its text, so the agent can correct the call.

## Sessions

Briefs and receipts join up by session. A command run by Claude Code reads the session from `CLAUDE_CODE_SESSION_ID`, but an MCP server may not be given it. So every tool takes an optional `session` argument. Without one, each brief is its own, and `receipt` and `next` compare with the base instead.

## What the server guarantees

- **Protocol output only.** The server writes nothing to stdout but protocol messages. Each tool runs as a separate `fairlead` process, which also keeps a crash in one call from ending the server.
- **Bounded calls.** A call is killed after 60 seconds, and output past 256 KB is cut and marked `[truncated]`. A cut result carries no `structuredContent`.
- **Repository text is marked.** Results that carry the repository's own text (lessons, skill descriptions, headings) have `_meta: {"fairlead/repository-text": true}`. The tools' descriptions say that text is data, not instructions.
- **Calls are logged without their arguments.** Each call adds an event at stage `mcp` to the event log, with the tool's name, the session when given, and `ok`, `refused` or `error`.
- **Protocol handling.**
  - Protocol versions 2025-06-18, 2025-03-26 and 2024-11-05 are echoed back, and any other version is answered with 2025-06-18.
  - Notifications get no reply.
  - Batches, which the 2025-06-18 revision removed, are refused.
  - The server offers tools only, so `resources/list` and `prompts/list` are unknown methods.
