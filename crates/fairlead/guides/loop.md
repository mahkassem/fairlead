# The change loop

Use this guide for every change in a repository that has a `fairlead.toml`: before the first edit, before you say the change is finished, and after.

## The commands, in order

```sh
fairlead brief <paths>   # before an edit: what the paths reach and what has to pass
fairlead done            # before you finish: the gate the change has to pass
fairlead receipt         # after it: the change against its brief, and the gate's state
fairlead next            # any time: the one step that's due
```

`<paths>` are the files or directories you expect to edit; a path that doesn't exist yet is a new file. A session has one brief: a later `fairlead brief` adds its paths to it.

## What the brief means

```text
brief b-b578d149  base 0346ca7  1 path
reaches  1 file imports them directly, 2 through the graph        graph importers, graph why
tests    1 test file                                              plan --files
checks   typecheck                                                plan --files
rules    size                                                     fairlead.toml [guard.*]
lessons  1 lesson                                                 .fairlead/lessons (fairlead lessons list)
skills   1 skill                                                  fairlead.toml [[skills.routes]]
done     unit, typecheck, guard                                   fairlead.toml [done], done --dry-run
next     edit, then: fairlead done
```

- `reaches`: the files that depend on the paths you named. A reach far wider than the task is a reason to stop and say so before editing.
- `tests`, `checks`: what will run for this change, and nothing else has to.
- `rules`: the guard presets that read these files. An edit that breaks one is refused as you write it.
- `lessons`: what earlier changes here taught. Read each one listed; `fairlead context <paths>` prints their text.
- `skills`: load the one named for the code you change.
- `done`: the gate's steps. The right-hand column is the command that checks each line.

## What the receipt and next mean

- `changed  3 files: 1 named in the brief, 1 in its reach, 1 outside`: an `outside` file is one the brief didn't count. Brief it (`fairlead brief <path>`) or say in the pull request why the change grew.
- `gate     passed for this tree`: the change is finished. `not run for this tree`: an edit came after the last pass, so run `fairlead done` again.
- `owed`: test files the gate left to CI. `fairlead guide gate` says what to do about them.
- `next`: do what the line says. It names `gh pr create --draft` until the gate passes and `gh pr ready` after.

## Also useful

```sh
fairlead find <words>            # search the lessons, skills, docs headings and declared names
fairlead test --explain <test>   # why a test is in the plan, or isn't
fairlead resume                  # a new session: where the last one on this branch stopped
fairlead learn --title "…" --path "src/x/**" --evidence <link> --source mistake   # record a lesson
```

## What to do next

- The gate failed, owes tests to CI, or a hook refused an edit: `fairlead guide gate`.
- A check is red on the pull request: `fairlead guide ci`.
- A session starts with `fairlead: setup has N open`: `fairlead guide setup`.
