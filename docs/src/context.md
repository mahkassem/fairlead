# Context and resume

`fairlead context` is the [brief](brief.md) with what an agent should read before the edit. `fairlead resume` is for a new session on a branch: where the last one stopped. *Since 0.8.0.*

```bash
fairlead context src/forms/form.ts   # the brief, then the lessons, skills, README and history
fairlead context --all src/forms     # everything, past the 120-line cap
fairlead context --json src/forms    # the same, as JSON
fairlead resume                      # the last brief on this branch, what changed, the gate and next
fairlead resume --json               # the same, as JSON
```

## Context

`context` makes the same brief `fairlead brief` makes for the paths, kept in the session's store the same way, and prints it. Then, in this order:

- **lessons:** each [lesson](lessons.md) the brief offered, with why, and its body. A lesson's body passed the secrets check when it was loaded.
- **skills:** each [skill](skills.md) the brief routed, with its description and the SKILL.md to load before editing.
- **docs:** for each named path, the first section of the nearest `README.md` up the tree, from the path's directory to the root; three READMEs at most, twelve lines of each. A section that looks like it holds a secret is left out.
- **history:** for each named file, the last three commits that touched it, `git log -3 --format='%h %ad %s' --date=short`. With the lessons, that answers most of "why is this code like this".

```text
brief b-575006d4  base e41ad7c  1 path
…
next     edit, then: fairlead done
lesson   blur  Validate on blur (named: src/forms/form.ts)
  Validate on blur.
  Not per key.
skill    forms  How forms validate.
  load it before editing: .claude/skills/forms/SKILL.md
docs     src/forms/README.md  Forms
  Every form posts through one helper.
history  src/forms/form.ts
  e41ad7c 2026-10-06 make the form add one
```

The text stops at 120 lines. A lesson, skill, section or history that doesn't fit is left out with everything after it, and the last line says how many: `… 4 more (fairlead context --all)`. `--all` lifts the cap. The JSON has no cap: `brief`, `lessons` (with `path` and `body`), `skills` (with `path`), `docs` and `history`.

## Resume

`resume` finds the brief to pick up: the session's own (`--session`, or `CLAUDE_CODE_SESSION_ID`), else the newest brief made on this branch by any session. A brief records the branch it was made on; one from before that counts when it has the same base. Then it prints, in 20 lines at most:

```text
resume   brief b-575006d4  base e41ad7c  updated 2026-10-06 14:02  (the newest on this branch)
  paths  src/forms/form.ts
changed  2 files since its base: 1 in the brief, 1 outside
  outside src/money.ts
gate     not run for this tree
lessons  1 added on this branch (fairlead lessons list)
  validate-on-blur                         Validate on blur
next     done: `fairlead done` hasn't passed for the tree as it is now
```

- **changed:** every file that differs from the brief's base, untracked files included, split into the ones the brief named and the rest.
- **gate:** the newest `fairlead done` run for the tree as it is now, as the [receipt](receipt.md) reads it.
- **lessons:** lesson files under `memory.dir` added since the base.
- **next:** the step [`fairlead next`](receipt.md) would name.

With no brief at all, it says so in one line and suggests `fairlead brief <paths>`.

## The SessionStart hook

`fairlead hooks install` adds a Claude Code `SessionStart` hook that runs `fairlead resume --hook` when a session starts, resumes, clears or compacts. It reads the hook's JSON on stdin (`session_id`, `cwd`) and answers with the resume text as context for the agent:

```json
{"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": "fairlead: where the last session on this branch stopped.\nresume   brief b-575006d4 …"}}
```

It prints nothing when there's no brief, or when anything fails, and always exits 0. `brief.resume = false` leaves the hook out. `fairlead migrate --write` adds it to hooks installed before. Fairlead's Codex and Gemini CLI hooks don't include it yet.
