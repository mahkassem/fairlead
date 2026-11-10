# Upgrade Fairlead

Use this guide when `fairlead setup --check` prints `open   binary`, when `fairlead doctor` warns that `package.json` asks for a later version than the binary, or when you are asked to move a repository to a new release.

## The commands, in order

```sh
fairlead --version            # the release that runs now
bun add -d fairlead@latest    # or npm, pnpm, yarn: the way the repository installed the old one
fairlead migrate              # what would change; nothing is written
fairlead migrate --write      # write the changes
fairlead doctor               # the binary, the versions the repository names, the hooks
fairlead config check         # the config still loads
fairlead setup --check        # nothing is open that wasn't before
```

Install the new release the way the old one was installed. A package dependency moves with its package manager, which also updates `package.json` and the lockfile. Otherwise: the shell or PowerShell installer, `brew upgrade fairlead`, or `cargo install --locked fairlead`.

## What the output means

```text
migrate: from ^0.4.2 (package.json) to 0.7.0
  would update .claude/settings.json: Fairlead's hooks: adds the brief nudge and the Stop hook
  would update fairlead.toml: `fairlead = "0.4"` becomes "0.6": the config uses `done`, which 0.6 added
  review (0.5.0): `plan.ignore` applies to source files too: ...
migrate: dry run; `fairlead migrate --write` makes these 2 change(s)
```

- `from … to …`: the release the repository was on, and where migrate read it. When it guesses wrong, name it: `fairlead migrate --from 0.4.2`.
- `would update <file>`: a change `--write` makes. Migrate changes only Fairlead's own hook entries, the git hook's command, the config's version floor, the version in `package.json` and the action's pin in a workflow.
- `review (X.Y.Z)`: something that release changed that a person has to decide. Settle each one before `--write`. A decision that isn't yours goes in the pull request, quoted.
- After `--write`, when it moved the version in `package.json`, it names the command that updates the lockfile (`bun install`, `pnpm install`, `yarn install` or `npm install`). Run it; migrate never edits a lockfile.
- `fairlead migrate --check` writes nothing and exits 1 when anything would change, 0 when the repository matches the installed release, and 2 when the config doesn't load.

## What migrate never does

It never installs anything, never adds hooks to a repository that has none (`fairlead setup` does), never changes a setting that decides what a plan selects, and never moves a pin to an older release.

## What to do next

1. Commit the result on a branch: the manifest and its lockfile, the hook settings, `lefthook.yml`, `fairlead.toml` and any workflow whose pin moved.
2. Run `fairlead done`, then open the pull request with the `review` lines and what you decided for each.
3. `fairlead setup --check` shows a new open item: the release added it. Its `fix:` line says what to run; `fairlead guide troubleshoot` has the rest.
4. The upgrade notes for each release are in the book's Upgrading page, oldest first.
