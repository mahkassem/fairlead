# Troubleshoot setup

Use this guide when `fairlead setup` or `fairlead setup --check` prints an `open` item and the `fix:` line under it didn't settle it. There is one entry per item id, in the checklist's order. After a fix, run `fairlead setup` again.

## binary
- Open when the running binary is older than the config's `fairlead` floor, or isn't the version `package.json` pins.
- Fix: install the pinned release the way the repository installs it (`bun install`, `npm ci`, the shell installer). Still open: another `fairlead` comes first on the PATH, and `fairlead doctor` prints the path of the one that runs.

## config
- Open when there is no `fairlead.toml`, or it loads with problems; the text is the first problem.
- Fix: with none, `fairlead init`. Otherwise `fairlead config check` lists every problem with its key; correct the key, and don't delete the file to start over.

## runners
- Open when a test file has no runner or more than one, or a runner's command names no `{files}`, so it runs its whole suite whenever one of its tests is selected.
- Fix: edit `[[tests.runners]]` in `fairlead.toml`: narrow a `match` or add an `exclude` until each test file has exactly one runner, and put `{files}` where the test files go in the command.

## proved
- Open when a runner ran its smallest test file and the test failed. `later` when the runner couldn't start: a need is unmet, or its tool is missing.
- Fix: run the runner's command on that file and read the error. A test that fails on a clean checkout is the repository's to fix: report it. A missing tool: install the dependencies (`npm ci`, `bun install`).

## hooks
- Open when an agent the repository shows has no Fairlead hooks.
- Fix: `fairlead hooks install`, with `--codex` or `--gemini` for those agents. `fairlead hooks status` says where each hook is.

## git-hook
- Open when `lefthook.yml` has no commit stage, or has one that is "written but not active": lefthook isn't in `.git/hooks`.
- Fix: `lefthook install`. No commit stage: `fairlead hooks install --git` first. No `lefthook` on the PATH: install it, then run both.

## agents
- Open when the Fairlead block in AGENTS.md or CLAUDE.md is missing or stale.
- Fix: `fairlead agents sync`. It writes only between its two markers. With `agents.write = "never"` the item is never open.

## ci
- Open when no workflow runs `fairlead ci` or the action. `later` on the `new` and `legacy` tracks.
- Fix: `fairlead ci workflow --write`, on a branch. It refuses to replace a workflow it didn't write; add a job to your own instead (`fairlead guide ci`).

## smoke
- Open when a runner whose suite is over 50 files, or which a `[[needs]]` entry covers, names no `smoke` set. Not required.
- Fix: name 1 to 3 fast, broad test files under that runner's `smoke` in `fairlead.toml`.

## gate
- Open when no `fairlead done` has passed on this machine in the last 30 days. Not required.
- Fix: `fairlead done`. When it fails: `fairlead guide gate`.

## measure
- Never open. `later` until the event log holds 10 gate runs: "the scorecard needs N more gate runs". Nothing to fix; then read `fairlead insights --score`.

## tests
- Guard profile only. Open when the repository now has tests a runner can run. Not required.
- Fix: `fairlead init --dry-run` prints the runner. Add it to `fairlead.toml` and remove `profile = "guard"`.
