# The done gate

Use this guide before you say a change is finished, when `fairlead done` fails or prints `owed to CI`, when a hook refuses an edit, and when you write the pull request.

## The commands, in order

```sh
fairlead done            # run the gate on the change as it stands
fairlead done --check    # has this exact tree passed? exits 0 only if it has
fairlead receipt --pr    # the block for the pull request body
```

## The budget and debts

The gate runs what fits in `done.budget_seconds` (300 by default), nearest the change first: test files the change edits, then tests that import a changed file, then tests further along an import chain. What the change reaches and doesn't fit isn't dropped: it is recorded as a debt, owed to CI, by runner and count. So the gate can pass on any machine, and nothing the change reaches is forgotten.

## What the output means

```text
done: unit: 41 of 380 reached test files run here; 339 owed to CI before merge: 339 past the 300 s budget
done: passed, 6 steps; owed to CI before merge: unit 339 test files (`fairlead receipt` lists them)
```

- `done: passed, N steps`: this tree passed. Any later edit makes a new tree, so run the gate again after one.
- `owed to CI before merge`: a debt. The gate still passed, and the CI that runs the plan on the pull request pays it. Don't raise the budget or run `fairlead done --full` to clear it unless you were asked to. Put `fairlead receipt --pr` in the pull request, so a reviewer sees what ran here and what is owed.
- `done: failed <step> in 0.5 s: <why>`: the step's command and output are above the line. Fix the cause and run `fairlead done` again. `could not start (exit 1 before any test ran)` means the runner didn't start, such as a service that isn't up, not that a test failed.
- `N files not run here: need "postgres" not met … proved in CI`: this machine lacks something the tests need. With `proved in`, the gate passes and that place proves them; say so in the pull request. With `owed to CI, where the merge check collects it`, the gate passes and the pull request's `settle` check fails if no job runs them. With neither, the gate can't pass until the need is met.
- `done: advice: …`: never fails the gate. Advice is for the pull request's notes or a follow-up, not for this change, unless it is what you were asked to do.

## The pull request

```sh
fairlead done && fairlead receipt --pr
```

Paste the block into the body. Open the pull request as a draft, `gh pr create --draft`, and mark it ready, `gh pr ready`, once the gate passes. Write "verified" or "tests pass" only after the gate passed on the commit you push: `fairlead pr check` fails a body that claims it with no passing receipt for the head's tree.

## When a hook refuses an edit

The refusal lists findings as `file:line rule: message`. The rule it names is what to fix: change the edit so the finding goes, then write again. It is not something to bypass. Don't write the file through a shell command instead, don't commit with `--no-verify`, and don't turn the rule off: the commit stage and CI run the same rules. If the rule is wrong for this code, say so and let a person change `fairlead.toml`.

The Stop hook is the same kind of message: it sends you back while the tree you leave hasn't passed, and names the one command to run.

## What to do next

- Passed: `fairlead receipt`, then `fairlead next`.
- Failed twice at the same step for a reason outside your change: stop and report the step, its `<why>` and what you tried.
- A check is red on the pull request after a local pass: `fairlead guide ci`.
- The gate or a hook said no and Fairlead itself looks wrong: `fairlead guide report`. The refusal stands while its owner looks.
