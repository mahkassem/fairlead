# The brief

`fairlead brief` answers, before the first edit, what a change to some paths will reach and what it takes to finish it. Every line names where its facts come from, so an agent can check any of them with the command on the right.

```bash
fairlead brief src/leave/Leave.ts            # one file
fairlead brief src/leave src/payroll/new.ts  # a directory, and a file that doesn't exist yet
fairlead brief src/leave --all               # every item, not the first five of each section
fairlead brief src/leave --json              # the same, as JSON
```

```text
brief b-b578d149  base 0346ca7  1 path
reaches  1 file imports them directly, 2 through the graph        graph importers, graph why
  src/b.ts                                     imports src/a.ts (import)
tests    1 test file                                              plan --files
  test/b.test.ts                               2 hops from src/a.ts
checks   typecheck                                                plan --files
rules    size                                                     fairlead.toml [guard.*]
lessons  1 lesson                                                 .fairlead/lessons (fairlead lessons list)
  leave-settles-at-sign-off                    Leave settles at sign-off (used: src/b.ts imports src/a.ts)
skills   none                                                     none routed yet (K4)
done     unit, typecheck, guard                                   fairlead.toml [done], done --dry-run
next     edit, then: fairlead done
```

## The sections

- **reaches:** the files that import a named path themselves, then how many depend on it through any chain of the import graph. `fairlead graph why A B` shows one chain.
- **tests** and **checks:** what the plan for these paths selects, with each test's first reason. It's `fairlead plan --files` run ahead of the change, and `fairlead test --explain FILE` explains any one of them.
- **rules:** the `[guard.*]` tables whose rules read a named path, such as `size`, `comments` or `migrations`. These are the rules the write hook will hold the edit to.
- **lessons:** the [lessons](lessons.md) whose scope covers a named path, a file a named path imports, or every change, each with why it was picked. It lists `memory.cap` (5) of them. A lesson file that can't be offered is named in a `warning` line. *Since 0.8.0.*
- **skills:** always empty for now. It's filled by skill routing on the roadmap without changing the format.
- **done:** the steps `fairlead done` will run, the same list as `fairlead done --dry-run`.

A path that isn't in the tree yet is a new file. Nothing reaches it, and the brief still lists the rules its location falls under. Each section shows its first five items, then how many more there are and the command that lists them. `--all` lifts the cap. Without it a brief stays under 40 lines.

## One brief per session

A brief is kept in `.git/fairlead/briefs/`, never committed, with the paths it named, the commit it started from and the plan it was made from. The receipt will compare what changed against it.

A brief belongs to the agent session that asked for it: `--session`, or the `CLAUDE_CODE_SESSION_ID` Claude Code sets for the commands an agent runs, which is the `session_id` its hooks receive. Under `brief.per = "session"` (the default), a second brief in the same session adds its paths to the first, so a change that grows is still one brief. A brief made from another base starts afresh. Under `"call"`, each brief stands alone and the newest one counts.

```toml
[brief]
per   = "session"   # session | call
nudge = true        # a note after the first edit made with no brief
```

## The note after an edit

An agent only reads a brief if it asks for one. `fairlead hooks install` adds a Claude Code `PostToolUse` hook on edits, `fairlead guard nudge`, which checks for a brief after the session's first edit. If the session has none, it adds one line to the agent's context saying how to get one. It says so once per session and never denies anything. A session that asked for a brief is never told. `brief.nudge = false` installs no such hook.

The note is recorded as a `nudge` event in `.git/fairlead/events.jsonl` with the session id, and that event is how the hook knows it already spoke.
