# Guard rules

The guard checks your project's own rules with one engine at every stage a
change passes through. This release has the engine, the check and commit
stages, and the first rule; the write stage (a Claude Code hook that stops a
write before it happens) and more presets follow.

| Stage | Command | Reads | Fails on |
| --- | --- | --- | --- |
| check | `fairlead guard check` | every tracked file, as it is on disk | any zero-tolerance finding, or a ratcheted count above the baseline |
| commit | `fairlead guard check --staged` | each staged file, at HEAD and in the index | a finding the staged change adds |

## Rules

Every preset is off until configured, and each one names the files it reads.

```toml
[guard]
baseline = "fairlead-baseline.json"   # the default
exclude = ["vendor/**"]               # tracked files no rule reads
deny = "added"                        # the default; "any" fails on every finding in a touched file
on_finding = "deny"                   # the default; "warn" shows the findings and lets the commit through
events = "local"                      # the default; "off" records nothing

[guard.size]
files = ["src/**/*.{ts,tsx}"]
exclude = ["src/generated/**"]
file_lines = 1000                     # a file over this many lines is a finding
ratchet = true                        # the default; false fails on any finding
```

A finding prints as `file:line rule: message`.

| Rule | Preset | Finds |
| --- | --- | --- |
| `file-length` | `guard.size` | A file over `file_lines` lines. A trailing newline doesn't start a line. |

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

`deny = "any"` makes a file you touch pass every rule instead: the commit stage
fails on any finding in a staged file, old or new. It suits a codebase with no
debt to carry; the default suits one that has some.

`on_finding = "warn"` lets the commit through and shows the findings instead of
stopping it. The check stage still fails on them in CI.

## Event log

Each commit-stage run appends one line to `.git/fairlead/events.jsonl`, inside
the git directory so it's never committed:

```json
{"at":"2026-09-26T19:04:11.221Z","stage":"commit","files":3,"decision":"deny","rules":["file-length"],"added":1,"ms":7.4,"fairlead":"0.4.0"}
```

It holds rule ids, paths, counts, decisions and timings, and never file
contents, comment text, commands, reasons or error messages, since any of
those can quote code. The log moves aside at 5 MB, keeping one old file.
Writing to it never fails a check, and `events = "off"` turns it off.
