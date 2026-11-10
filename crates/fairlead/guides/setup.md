# Set a repository up

Use this guide when a repository has no `fairlead.toml`, when a session starts with `fairlead: setup has N open`, or when `fairlead doctor` ends with `setup: N open`.

## The commands, in order

```sh
fairlead setup            # apply what is local and reversible, then print the checklist
fairlead setup --check    # print the checklist and change nothing
fairlead setup --dry-run  # print what setup would change
fairlead setup --json     # the checklist as JSON: the track, the items, how many are open
```

Run `fairlead setup` at the repository's root. It writes a first config when there is none, installs the hooks for each agent the repository shows, adds the git hook, syncs the block in AGENTS.md and CLAUDE.md, and runs each runner's smallest test file once to prove the runner starts. It never writes or edits a CI workflow, never deletes anything and never changes a `fairlead.toml` that already exists.

## What the output means

```text
fairlead setup    track: new (no tests yet, 6 commits)
  ok     binary     0.20.0 on PATH
  fixed  config     fairlead.toml, no problems, guard profile
  fixed  hooks      Claude Code: edit guard, brief nudge, stop gate, session start
  fixed  git-hook   the commit stage is in lefthook.yml, and lefthook runs it
  fixed  agents     the block is current in AGENTS.md, CLAUDE.md
  open   gate       no passing `fairlead done` on record
         fix: fairlead done
  ok     tests      no tests a runner here can run yet
  later  ci         no workflow runs `fairlead guard check`; the new track leaves it for later
1 open. Fix it and run `fairlead setup` again. `fairlead setup --check` shows this list any time.
```

Each line is a state, an item's id and what was found.

- `ok`: nothing to do.
- `fixed`: setup changed it just now. Read `git status`, then commit what it wrote.
- `open`: yours to settle. Every open item has exactly one fix, on the `fix:` line under it: one command or one config edit. Do that and nothing else.
- `later`: not due yet, or not provable on this machine; the text says why. Leave it.

The exit code is 1 while a required item is open, else 0.

## The track

The first line names the track setup detected and why. The track decides which items are required. It is never written to the config.

- `new`: no tests yet and under 20 commits. The guard and the hooks; CI is `later`.
- `mvp`: some tests and a short history. The runners and the gate; CI is open but not required.
- `ongoing`: CI already runs tests or deploys. Every item, and CI is required.
- `legacy`: a long history with few tests, or a stack the test planner doesn't read. The guard first; CI is `later`.

When the track is wrong, name it for one run: `fairlead setup --track mvp` (or `new`, `ongoing`, `legacy`).

## What to do next

1. Apply each `fix:` line, top to bottom, then run `fairlead setup` again, until no line says `open`.
2. Commit what setup wrote on a branch: `fairlead.toml`, the agent's hook settings, `lefthook.yml`, AGENTS.md and CLAUDE.md.
3. A fix that doesn't settle its item: `fairlead guide troubleshoot` has an entry for each id.
4. Then make a change: `fairlead guide loop`.
