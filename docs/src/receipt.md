# The receipt and next

After a change, `fairlead receipt` compares what changed with the session's [brief](brief.md) and the [done gate](done.md). `fairlead next` says which step the change is waiting for.

```bash
fairlead receipt                    # compare the working tree with the session's brief
fairlead receipt --out receipt.md   # also write it to a file a commit or pull request can carry
fairlead receipt --json             # the same, as JSON
fairlead next                       # the one step that's due
```

```text
receipt for brief b-b578d149  base 72087d5..worktree
changed  3 files: 1 named in the brief, 1 in its reach, 1 outside
  outside  src/auth.ts  adds 1 test (test/auth.test.ts)
tests    planned now 2, briefed 1: 1 added by files outside the brief
gate     passed for this tree at 16:01 (1 step) in 9 s
next     ready: the gate passed and the receipt is written; `gh pr ready` if the pull request is a draft
```

## Inside, reach and outside

Each changed file falls into one of three groups:
- **named in the brief:** the brief listed it.
- **in its reach:** the file depends on a path the brief named, so the brief already counted it.
- **outside:** neither. This is the drift a receipt is for. A change that named billing and also edited auth says so, with the tests the extra file adds to the plan.

The tests line compares the plan for what changed with the plan the brief was made from. The gate line reads the newest `fairlead done` run for the tree as it is now, so an edit after a pass shows as not run. *Since 0.7.1:* a pass that excused a failure a [`[[quarantine]]` entry](plan.md#tests-that-lie-on-one-platform) expects says how many steps were held, as `(3 steps, 1 held)`, and the receipt's JSON carries the count in `gate.held`, so it doesn't read as a clean pass.

With no brief for the session, the receipt still lists every changed file and the gate's state, and says there was nothing to compare against.

A receipt is kept in `.git/fairlead/receipts/`, never committed, beside the brief it belongs to. `--out` writes it to a file: JSON for a `.json` name, the text otherwise.

## next

One line, from the state of the change:

| State | next |
|---|---|
| Nothing has changed | nothing due |
| Changed, no brief for the session | `fairlead brief <paths>` |
| Briefed, the gate hasn't passed for this tree | `fairlead done`, and `gh pr create --draft` if no pull request is open yet |
| The gate failed | the failing step's command, then `fairlead done` |
| The gate passed, no receipt for this tree | `fairlead receipt`, then `gh pr ready` |
| The gate passed and the receipt is written | `gh pr ready` if the pull request is a draft |

The session comes from `--session` or `CLAUDE_CODE_SESSION_ID`, as for the brief.

*Since 0.8.0:* `next` also says when to open the pull request as a draft and when to mark it ready, so a CI that runs only fast checks on a draft runs the rest once it's marked ready. Fairlead never runs `gh` and doesn't ask whether a pull request is open, so the line reads right either way. A [`[[guard.commands]]` rule](guard.md#commands) can refuse `gh pr create` without `--draft`.
