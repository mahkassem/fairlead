# The receipt and next

After a change, `fairlead receipt` compares what changed with the session's [brief](brief.md) and the [done gate](done.md). `fairlead next` says which step the change is waiting for.

```bash
fairlead receipt                    # compare the working tree with the session's brief
fairlead receipt --out receipt.md   # also write it to a file a commit or pull request can carry
fairlead receipt --json             # the same, as JSON
fairlead receipt --pr               # a block for the pull request body
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

## Stages and platforms

*Unreleased:* a pass on a laptop says nothing about the code on another OS, or about the stages it didn't run. The receipt says, after the gate line, where each step of the newest `fairlead done` run on this tree ran, by CI stage and platform, and plainly what it didn't prove:

```text
gate     failed for this tree at lint
ran      draft, linux/x86_64: lint (failed)
ran      ready, linux/x86_64: unit
not run  never run on this tree: types
not run  unverifiable here: native-build needs macos; proved in CI (macos job)
```

- **ran:** each step by its stage (its `from`, or `ready` for a runner and `draft` for a check) and the `os/arch` it ran on, marked `(failed)` or `(held)`. A run an earlier release recorded says `stage not recorded, platform not recorded`.
- **never run on this tree:** steps the gate plans now that its newest run didn't reach, such as those after a failure without `--keep-going`; with no run, every one.
- **unverifiable here:** steps an unmet [`[[needs]]` entry](done.md#needs) left out, `needs macos` for an `os` need and `needs "postgres"` for any other, and steps a [`[[quarantine]]` entry](plan.md#tests-that-lie-on-one-platform) held, each with where it's proved.

The JSON carries the same as `gate.ran` (with `stage`, `os` and `arch`), `never_run` and `unverifiable`; each is left out when empty, and a receipt without them still reads.

`fairlead receipt --pr` prints the receipt as a block for the pull request body, which `pr check` reads to tie the body's claims to a pass for the head's tree, and `ci report` shows in its summary ([Claims and receipts](pr.md#claims-and-receipts)).

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
| The gate's steps passed and a whole suite waits (*Since 0.15.2*) | `fairlead done --full`, with what holds the suite, why, and the other way forward ([Whole suites](plan.md#the-projects-phase)) |
| The gate passed, no receipt for this tree | `fairlead receipt`, then `gh pr ready` |
| The gate passed and the receipt is written | `gh pr ready` if the pull request is a draft |

The session comes from `--session` or `CLAUDE_CODE_SESSION_ID`, as for the brief.

*Since 0.8.0:* `next` also says when to open the pull request as a draft and when to mark it ready, so a CI that runs only fast checks on a draft runs the rest once it's marked ready. Fairlead never runs `gh` and doesn't ask whether a pull request is open, so the line reads right either way. A [`[[guard.commands]]` rule](guard.md#commands) can refuse `gh pr create` without `--draft`.

*Unreleased:* when a [`fairlead ci ledger`](ci.md#is-this-red-mine-fairlead-ci-ledger) for HEAD is under 30 minutes old, `next` adds a second line, `ci: 2 of 3 failures inherited: red on main since abc1234 (2026-11-07); 1 own (ledger 4 min old; …)`. It reads only that cached file; `next` still never runs `gh` or reaches the network. `ci ledger` is the command that reads the GitHub API, and it may run `gh auth token` for a token.
