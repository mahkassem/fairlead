# Guard rules

The guard checks your project's own rules with one engine at every stage a
change passes through. This release has the engine, the check and commit
stages, and the size and comment presets; the write stage (a Claude Code hook
that stops a write before it happens) and more presets follow.

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

Each commit-stage run appends one line to `.git/fairlead/events.jsonl`, inside
the git directory so it's never committed:

```json
{"at":"2026-09-26T19:04:11.221Z","stage":"commit","files":3,"decision":"deny","rules":["file-length"],"added":1,"ms":7.4,"fairlead":"0.4.0"}
```

It holds rule ids, paths, counts, decisions and timings, and never file
contents, comment text, commands, reasons or error messages, since any of
those can quote code. The log moves aside at 5 MB, keeping one old file.
Writing to it never fails a check, and `events = "off"` turns it off.
