# Guard rules

The guard checks your project's own rules with one engine at every stage a
change passes through: as an agent writes a file, at commit, and in CI.

| Stage | Command | Reads | Fails on |
| --- | --- | --- | --- |
| write | `fairlead guard hook`, run by Claude Code | the file an edit would leave, before it's written | a finding the edit adds; see [The write hook](#the-write-hook) |
| check | `fairlead guard check` | every tracked file, as it is on disk | any zero-tolerance finding, or a ratcheted count above the baseline |
| commit | `fairlead guard check --staged` | each staged file, at HEAD and in the index | a finding the staged change adds |

The check and commit stages also run the migration rules and the external rules configured for them.

## Rules

Every preset is off until configured, and each one names the files it reads.

```toml
[guard]
baseline = "fairlead-baseline.json"   # the default
exclude = ["vendor/**"]               # tracked files no rule reads
findings = "added"                    # the default; "all" counts every finding in a touched file
on_finding = "deny"                   # the default; "warn" shows the findings and lets the commit through
events = "local"                      # the default; "off" records nothing

[guard.size]
files = ["src/**/*.{ts,tsx}"]
exclude = ["src/generated/**"]
file_lines = 1000                     # a file over this many lines is a finding
function_lines = 120                  # a function over this many of its own lines
ratchet = true                        # the default; false fails on any finding

[guard.cite]
file-length = "style guide, section 6"   # appended to that rule's messages
```

A finding prints as `file:line rule: message`, with the rule's `cite` after it
when there is one.

| Rule | Preset | Finds |
| --- | --- | --- |
| `file-length` | `guard.size` | A file over `file_lines` lines. A trailing newline doesn't start a line. |
| `function-length` | `guard.size` | A function over `function_lines` of its own lines. |
| `block-length` | `guard.comments` | A comment block longer than its context allows. |
| `density` | `guard.comments` | Comment lines over a share of comment and code lines. |
| `history` | `guard.comments` | A date, a name, a narrative phrase, or a measurement beside "measured". |
| `item-code` | `guard.comments` | A ticket or item reference outside the pointer form. |
| `agent-instruction` | `guard.comments` | A comment that addresses its next editor. |
| `block-marker` | `guard.comments` | A continuation line of a multi-line `/* */` that doesn't start with `*`. |
| `test-file-name` | `guard.test_names` | A test file whose name doesn't match `file`. |
| `test-title` | `guard.test_names` | A test title that matches `titles_without`. |
| `citation` | `guard.citations` | A pointer in a comment that names no heading in `headings_in`. |
| `migration-edit` | `guard.migrations` | A change, move or deletion of a migration that exists at the base. |
| `migration-prefix` | `guard.migrations` | Two migrations with the same leading number. |
| your `id` | `guard.external` | Whatever the command reports. |

## Function length

A function's own lines run from its first line to its last, minus the lines of
the functions directly nested in it, so a long callback is charged to itself
and not to every function around it. Declarations, expressions, arrows,
methods, constructors, getters, setters and generators all count, and a
function starts at an `export` or decorator in front of it. A callback passed
straight to a test hook isn't measured itself, only what it nests; the hooks
are `test_hooks`, by default `describe`, `test`, `it` and the `before` and
`after` hooks, found through chains such as `it.each([...])(...)` and
`test.only(...)`. JavaScript and TypeScript only, from a syntax tree.

## Comments

```toml
[guard.comments]
files = ["**/*.{ts,tsx,sql,yml}"]
tests = ["**/*.test.ts"]                  # take the test limits
migrations = ["db/migrations/*.sql"]      # one-line header, no density limit
block_length = { source = 8, test = 10, header = 12, inline = 2, migration = 1 }
density = { source = 0.25, test = 0.20 }
history = { dates = true, names = ["Ada"], phrases = ["used to", "no longer"], measured = true }
item_codes = { pattern = '(?-u:\b)[A-Z]+-[0-9]+(?-u:\b)', pointer = true }
agent_phrases = ['(?i)(?-u:\b)you must(?-u:\b)']
block_marker = true
ratchet = false                           # the default: any finding fails
```

The comment syntax comes from the extension: `//` and `/* */` for JavaScript
and TypeScript, `--` for SQL, `#` for YAML, TOML and shell.

- **Blocks.** A block is a run of lines that are comments once trimmed, block
  comments included. Its context is the first that has a limit: `migration`,
  `inline` (its first line is indented and the nearest non-blank line above is
  code), `header` (it starts on line 1), `test`, then `source`.
- **What a comment says** is read from its flattened text: each line's marker
  stripped and whitespace collapsed, so a phrase split across a wrap still
  matches. Names and phrases match as whole words; phrases in any case. A
  date is one from 2000 to 2099 written `YYYY-MM-DD`.
- **Comments after code**, such as `x() // why`, are found from the syntax tree,
  so `//` in a string, a template or JSX text is never taken for one. They're
  checked for what they say, and don't count as blocks or towards density.
- **The pointer form**: with `pointer = true`, references are allowed as
  `(ABC-1)` or `(ABC-1, ABC-2)` followed by a full stop or the end of the
  comment. Each other reference is a finding, once per comment.
- **Patterns** are Rust regular expressions, where `\b` and `\d` are Unicode
  aware. Write `(?-u:\b)` and `[0-9]` for the ASCII behaviour most other
  linters have, and `(?i)` for any case.

## Test names

```toml
[guard.test_names]
files = ["test/**/*.test.ts"]
file = '^[a-z0-9]+(-[a-z0-9]+)*\.test\.ts$'   # the file's name must match
titles_without = '(?-u:\b)[A-Z]+-[0-9]+(?-u:\b)'  # no title may match
title_calls = ["describe", "test", "it"]        # the default
```

A title is the first argument of a title call, found through chains such as
`it.only(...)`, when it's a string or a template with no `${...}`.

## Citations

```toml
[guard.citations]
files = ["src/**"]
pattern = '\((?P<code>[A-Z]+-[0-9]+)\)'   # a pointer; the name is the `code` group
headings_in = "docs/decisions.md"
```

Each pointer in a comment, a block or after code, has to name a heading in the
Markdown file: a heading names the first word of its text, so `## ABC-12: why`
names `ABC-12`. The file is read once per run, from the working tree.

## Migrations

```toml
[guard.migrations]
files = ["db/migrations/*.sql"]
immutable = true                         # the default
base = "origin/main"                     # optional
unique_prefix = { allow = [["089_a.sql", "089_b.sql"]] }
```

A database that ran a migration won't run it again, so once a migration exists
it may not change, move or go. "Exists" means present at the base: the merge
base with `base` when it's set. Without it, the commit stage compares with HEAD
and the check stage needs `--base REV`, and says so. At the commit stage a base
that can't be found, before the first commit or with the ref not fetched,
falls back to HEAD. `unique_prefix` makes each
file's leading number unique, apart from the listed groups; the commit stage
fails only on a number the commit newly shares.

## Commands

```toml
[[guard.commands]]
match = '(^|\s)git push --force(\s|$)'
reason = "Open a pull request instead; a forced push rewrites what others have."
```

A shell command an agent may not run, with the reason it's told. The write
stage's hook reads these; the check and commit stages don't run commands.

## External rules

```toml
[[guard.external]]
id = "design-lint"
command = ["npm", "run", "lint:design"]
stages = ["check"]                        # the default; also "commit"
ratchet = false                           # the default
```

Any tool that prints findings as `file:line message` or `file:line:column
message`, one per line, becomes a rule with your `id`; other lines are ignored.
A tool that exits non-zero and prints no findings fails the check, since it
couldn't run. At the check stage the command runs as written. At the commit
stage `{files}` expands to the staged files and every finding in them counts,
since there's no before side to compare with. `write` isn't a stage an
external rule can run at: when the hook runs, the file isn't written yet.

## The write hook

```bash
fairlead hooks install         # the Claude Code hooks, and the git hook if lefthook is set up
fairlead hooks install --git   # only the git hook, making lefthook.yml if there's none
fairlead hooks install --codex # the same hooks for Codex, in .codex/hooks.json
fairlead hooks status          # where each is, and whether uninstall can restore the file exactly
fairlead hooks uninstall       # take them out again
fairlead doctor                # the hooks, the binary on the PATH, and what the event log recorded
```

`hooks install` adds three Claude Code hooks:

| Hook | Runs | Does |
|---|---|---|
| `PreToolUse` | `fairlead guard hook` | The write stage below: denies an edit that breaks a rule, or adds a note |
| `PostToolUse` | `fairlead guard nudge` | Once per session, after an edit made with no brief, says how to get one ([The brief](brief.md#the-note-after-an-edit)); off with `brief.nudge = false` |
| `Stop` | `fairlead guard stop` | Sends the agent back while the tree it leaves hasn't passed `fairlead done` ([The Stop hook](done.md#the-stop-hook)); off with `done.on_stop = "off"` |

The hooks go where `hooks.claude` says: `"shared"` (the default) is the
committed `.claude/settings.json`, so everyone who clones the repository and
every agent session is guarded; `"local"` is `.claude/settings.local.json`,
for you alone. `--shared` and `--local` choose for one run. The write hook
runs on `Edit`, `Write` and `MultiEdit`, and on `Bash` too when there are
`[[guard.commands]]`. Where `fairlead` isn't installed the hooks do nothing,
so a teammate without it can still work.

When `package.json` lists Fairlead as a dependency, install writes hooks that
run the project's own copy rather than one on the PATH, which a package
install doesn't provide. The Claude Code hook calls the binary the npm package
unpacked, `node_modules/fairlead/node_modules/.bin_real/fairlead`, and falls
back to the package runner the lockfile names (`bun x`, `pnpm exec`, `yarn`,
else `npx --no-install`), which costs that runner's start-up on every edit.
The lefthook entry runs through the same runner. The npm package unpacks the
binary in a `postinstall` script run with `node`; where that script isn't
allowed to run, or there's no `node`, the runner fetches it on first use.

Before an edit, the hook works out what the file would hold and lints it
with the same presets as the other stages. When the edit adds a finding it
denies the edit, and the agent reads the findings as the reason, fixes them
and writes again. With `on_finding = "warn"` it lets the edit through and
passes the findings to the agent as a note instead. It never answers
"allow", which would also skip your own permission prompt. An edit to a
migration that already exists is denied outright. To tell which exist
without starting `git`, the hook reads HEAD's commit from the ref files and
keeps the paths under the migration directories for it in
`.git/fairlead/head-paths/`, for the last four commits; `guard check` and
the commit stage fill it for the current HEAD, and the hook fills it with
one `git ls-tree` when HEAD has moved since.

The hook lets a call go ahead, and says so in the event log, when it can't
decide: the edit's old text isn't in the file or occurs more than once, the
tool input has a shape it doesn't know, the file is outside the project, not
UTF-8 or over 256 KB, the config has a problem, or it runs out of its own
time, `guard.budget_ms` (40 by default). The check and commit stages still
catch whatever it lets through.

Install keeps a manifest in `.git/fairlead/backups/` with the settings file's
original bytes and the bytes it wrote. Installing twice changes nothing.
Uninstall puts the original back byte for byte when nobody changed the file
since; otherwise it removes only Fairlead's entries, keeps everything else,
and says the formatting may differ. On a fresh clone, where there's no
manifest, it removes the entries by their command.

### Codex

*Unreleased:* `fairlead hooks install --codex` writes the same three hooks to
`.codex/hooks.json`, which Codex reads in the same shape; `status` and
`uninstall` take `--codex` too. Codex edits files with one tool,
`apply_patch`, whose patch can add, change, move or delete several files at
once. The write hook reads the patch the way Codex applies it, lints each
file as the patch would leave it, and answers with one deny that lists every
finding. Moving or deleting a migration that already exists is denied, as
editing one is. A patch typed into the shell as `apply_patch <<'EOF'` is read
the same way. A patch that doesn't parse, or whose lines aren't in the file,
goes ahead: Codex refuses it on its own.

Codex starts a hook in the session's directory, so the Codex entries change
to the repository's root first. It runs a project's hooks only once you trust
the project and approve them: it asks when it starts, and `/hooks` in Codex
lists them.


The commit stage runs through [lefthook](https://github.com/evilmartians/lefthook):
install adds a `fairlead-guard` command running `fairlead guard check --staged`
(through the package runner where Fairlead is a package dependency)
to `pre-commit` in `lefthook.yml` (or `lefthook.yaml`, `.lefthook.yml`,
`.lefthook.yaml`, whichever is there). It adds lines at the indentation around
them, under `commands`, or as an item where the config uses `jobs`, and never
rewrites the rest, so comments and layout stay. A `pre-commit` or `commands`
written on one line (`pre-commit: {}`) is refused, to be added by hand.

Without `--git`, install adds the git hook only where a lefthook config already
exists. lefthook runs the command once `lefthook install` has put it in
`.git/hooks`; `status` and `doctor` say when it hasn't. Uninstall restores the
file byte for byte when nobody changed it since, else removes the two lines
and any `commands`, `jobs` or `pre-commit` they leave empty, and a config
install made is removed.

## Doctor

`fairlead doctor` reads the event log and reports the newest 500 write-hook
calls by decision, the rules that denied, the hook's p50 and p95 time, any
calls that ran out of time or hit an error, and the commit stage's runs. It
also says where each hook is installed, whether lefthook will run the git one,
and whether `fairlead` is on the PATH, since the hooks call it by name, or
that they run the project's own copy.

## Checking the guard against your linter

When the guard replaces a linter you already run, check that the two agree
before you switch:

```bash
your-linter --list > theirs.txt                 # one finding per line: file:line rule
fairlead guard compare theirs.txt               # fails on any difference
fairlead guard compare theirs.txt --map max-lines=file-length --rules file-length
```

`compare` reads `file:line rule`, with an optional `:column` and anything after
the rule, and matches findings on file, line and rule, so the wording of
messages doesn't matter. `--map` renames the other linter's rules to
Fairlead's, and `--rules` compares only the ones named. It lists up to 20
findings each side has that the other doesn't.

`fairlead guard bench --since REV` replays your history through the write
hook: for each commit since `REV` on HEAD's first-parent line (a merge counts
as the change it brought in), it sends each changed file the guard reads to
`fairlead guard hook` as a write, in a temporary worktree with today's config.
It reports the edits, p50, p95 and the slowest wall time, process start
included, and what the hook decided. `--limit` stops after that many edits
(200 by default), and `--p95-under MS` fails when p95 is over it. The main
event log is left alone.

## The ratchet

A ratcheted rule is for debt you already have: it fails only when a file has
more findings of that rule than the baseline allows.

```bash
fairlead guard check --write-baseline   # record today's counts
fairlead guard check                    # fails if any file/rule count rises
fairlead guard check --list             # every finding, held or not
```

The baseline is a JSON file of counts by file and rule, meant to be committed.
A count can fall freely; the check says when one did, and `--write-baseline`
lowers the file to match. Counts for a rule that is no longer ratcheted, or no
longer configured, are ignored.

## Only what a change adds

The commit stage judges a change by what it adds, so fixing one line in a file
with an old finding isn't blocked by it; the check stage still reports it.
Each staged file is linted as it is at HEAD and as it is in the index, never
the working tree, and the two lists are compared:

- Most findings compare by rule, message and what they're about (a comment's
  text, a function's name), never by line, so a finding that only moved isn't
  new and a second identical one is.
- A measured finding, such as a long file, is new when the thing crosses its
  limit or grows while over it. Editing inside a long file without making it
  longer is never a finding.

A renamed file is compared with its old self, and before the first commit
everything staged is new.

`findings = "all"` makes a file you touch pass every rule instead: the commit stage
fails on any finding in a staged file, old or new. It suits a codebase with no
debt to carry; the default suits one that has some.

`on_finding = "warn"` lets the commit through and shows the findings instead of
stopping it. The check stage still fails on them in CI.

## Event log

Each write-stage and commit-stage run appends one line to
`.git/fairlead/events.jsonl`, inside the git directory so it's never
committed:

```json
{"at":"2026-09-26T19:04:11.221Z","stage":"commit","files":3,"decision":"deny","rules":["file-length"],"added":1,"ms":7.4,"fairlead":"0.4.0"}
```

It holds rule ids, paths, counts, decisions and timings, and never file
contents, comment text, commands, reasons or error messages, since any of
those can quote code. The log moves aside at 5 MB, keeping one old file.
Writing to it never fails a check, and `events = "off"` turns it off.
