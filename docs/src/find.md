# Find

`fairlead find` searches what a project knows in one place: its [lessons](lessons.md), its routed [skills](skills.md), the headings of its Markdown docs, and the classes and functions its PHP, Java and Kotlin files declare. Hits near the files the session's [brief](brief.md) named come first. *Since 0.8.0.*

```bash
fairlead find retry budget          # the 20 best hits, best first
fairlead find retry budget --json   # the same, as JSON
fairlead find --symbol Invoice      # where a name is declared
fairlead find ledger --limit 50     # up to 50 hits
```

Each hit is one line: its kind, its name, where it is and its score.

```text
lesson  Round once              .fairlead/lessons/money-rounding.md     4.12
doc     Rolling back a release  docs/deploy.md#rolling-back-a-release   2.87
skill   web-forms               .claude/skills/forms/SKILL.md           1.94
symbol  App\Models\Invoice      app/Models/Invoice.php                  1.31
```

## What it searches

The index is built in memory on every run from the tree as it is, so there's nothing to keep up to date.

| Kind | Name | Text | Path |
| --- | --- | --- | --- |
| `lesson` | its title | its id and body | the lesson file |
| `skill` | its name | its description | the SKILL.md |
| `doc` | the heading | the first paragraph of its section | the file and the heading's `#anchor` |
| `symbol` | the qualified name | | the declaring file |

Docs are the `#` to `####` headings of every `.md` file in the tree, outside code fences and front matter. Files the graph's tree scan ignores are left out, and so are `node_modules`, `vendor`, `.git`, `target`, `dist` and `build`, and the lessons' own folder (`memory.dir`). Symbols come from the declaration indexes the [graph](graph.md) already builds for PHP, Java and Kotlin. Those indexes keep no line numbers, so a symbol's path is its file. TypeScript, Python and Go declarations aren't searched yet.

A lesson or skill file that can't be read is left out with a warning on stderr; `fairlead lessons check` names it.

## How hits are ranked

The ranking is BM25 (k1 = 1.2, b = 0.75). A word matches the same word, so `retryBudget` and `max_retry_count` are found by `retry` as well as by the whole name, and a handful of common words such as "the" and "is" are ignored. A word in the name counts twice as much as one in the text, and a rare word counts for more than a common one.

### Nearness to the brief

When the session has a brief (`--session`, or `CLAUDE_CODE_SESSION_ID`, as for `fairlead brief`), each hit's score is multiplied by `1 + 1 / (1 + d)`, where `d` is how many import edges, in either direction, separate it from the nearest path the brief names:

| Distance | Multiplier |
| --- | --- |
| a path the brief names | 2 |
| 1 | 1.5 |
| 2 | 1.33 |
| 3 | 1.25 |
| further, or no brief | 1 |

A doc or symbol is as near as its file. A lesson or skill is as near as the nearest file its scope covers within those three hops, so a lesson scoped to `src/billing/**` is lifted by a brief for a file that imports billing code. A scope of only `always = true` covers no file, so it isn't lifted. The walk stops at a [`graph.barrier`](graph.md#barriers), as the brief's does.

The text output says when a brief was used. In the JSON, `brief` names it and is `null` without one.

## Finding a declaration

`fairlead find --symbol NAME` lists only declarations named exactly `NAME`, either in full (`App\Models\Invoice`, `com.shop.Order`) or by their last part (`Invoice`, `Order`). Case-sensitive matches are listed if there are any; otherwise the ones that match when case is ignored. A partial name finds nothing.

## Options

- `--limit N`: show up to `N` hits, from 1 to 50 (20 by default).
- `--json`: print `{ "query", "brief", "hits": [{ "kind", "name", "path", "score", "snippet" }] }`. The snippet is the start of the lesson's body, the skill's description, the heading's paragraph or the symbol's name.
- `--session ID`: rank by this session's brief instead of `CLAUDE_CODE_SESSION_ID`'s.
- `--set KEY=VALUE`: override a config value for this run.

A query that matches nothing prints `nothing matches` and exits 0. A config with problems is searched as read, with a warning.

## Size

On a synthetic project with 650 lessons, 220 docs headings and 200 source files, a query answered in a median of 72 ms (77 ms with a brief, slowest 117 ms) on a 4-core Linux machine, scan and process start included. The index has no dependency of its own. If a corpus grows past 10,000 entries or a query past 200 ms, a persistent index is worth a look.
