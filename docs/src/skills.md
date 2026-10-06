# Skills

A skill is a SKILL.md in the open format agents already read: a name, a description and the instructions. Fairlead doesn't change the file. It routes it: `[[skills.routes]]` in `fairlead.toml` says which code each skill applies to, and the brief lists the skills a change reaches, the way the plan lists its tests. *Since 0.8.0.*

```toml
[skills]
imports = 1      # hops along what a changed file imports
importers = 0    # hops along the files that import it; off by default
cap = 8          # skills a brief lists before "N more"

[[skills.routes]]
skill = ".claude/skills/forms/SKILL.md"
paths = ["src/forms/**"]

[[skills.routes]]
skill = ".claude/skills/house-style/SKILL.md"
always = true
```

A route needs a scope: `paths` (globs), `modules` (module names) or `always = true`. Each skill is routed once. Fairlead reads only the skill's `name` and `description` from its front matter, falling back to its directory's name.

## How a change picks its skills

Skills are picked by the same router as [lessons](lessons.md):

1. **named:** the scope covers a path the change names;
2. **used:** the scope covers a file a named path imports, within `skills.imports` hops. The walk stops at a [`graph.barrier`](graph.md#barriers);
3. **importer:** only with `skills.importers` above 0, the scope covers a file that imports a named path;
4. **always**, then the **fallback** when the plan runs everything.

Imports are the default because a file needs the contracts it uses, and nothing imports a test, so a test's skills are only reachable forward. On one project's history, routing along imports caught slightly more of the rules a change needed than routing along importers did, and the two together caught no more than imports alone. Importers stay one setting away, for a repository whose history says otherwise.

The brief's skills row lists `skills.cap` (8) skills, each with its description and why it was picked. A route whose SKILL.md can't be read is named in a `warning [bad-skill]` line.

## While the agent works

The `PostToolUse` hook `fairlead hooks install` adds does two more things when routes exist:

- **After an edit**, it names the skill whose scope covers the edited file, with its description, once per skill per session.
- **When a skill is loaded**, through Claude Code's `Skill` tool or a Gemini CLI `read_file` of a SKILL.md, it records a `use` event in `.git/fairlead/events.jsonl`. Each brief records an `offer` event with what it listed. Together they give routing a hit rate: what was offered, what was used, and what was used without being offered.

Codex has no hook that sees a file being read, so a skill it loads isn't seen; its use is unmeasured rather than counted as a miss.

## Starting from rule files: import rules

A team that already scopes rules by path, in Claude Code's `.claude/rules/*.md` (front matter `paths`) or Cursor's `.cursor/rules/*.mdc` (`globs`, `alwaysApply`), can start from them:

```sh
fairlead import rules .claude/rules            # dry run: prints each skill and the routes
fairlead import rules .claude/rules --write    # writes them
```

Each rule file becomes `.claude/skills/<name>/SKILL.md`, named after the file (a file in a subdirectory gets the directory in its name), with `name` and `description` front matter and the rule's body unchanged. The description is the rule's own, or the opening of its body: sentences, without bold or italic markers, until they reach 40 characters, on one line of at most 200. Its scope becomes a `[[skills.routes]]` block: `paths` from `paths` or `globs`, given as a list or a comma-separated string, or `always = true` for `alwaysApply: true`. A Cursor glob without a slash, such as `*.tsx`, matches at any depth, so it's written as `**/*.tsx`. A Claude Code rule with no `paths` is one Claude Code loads in every session, so it becomes `always = true`. A Cursor rule with no `globs` and no `alwaysApply` is one Cursor applies only on request, so it's skipped with "no scope".

Without `--write` nothing is written. With it, the skills are written and the route blocks are appended to the end of `fairlead.toml` under a comment naming the command; the existing text isn't touched. A rule whose skill is already routed is left alone, so running it again changes nothing. A different file already at a skill's path stops the run before anything is written. The rule files stay where they are, and their agents still load them: remove them once the skills replace them.

## Measuring routing: skills eval

```sh
fairlead skills eval                  # the last 500 first-parent commits
fairlead skills eval --since 2026-01-01 --json
```

`skills eval` scores the router on the repository's history. A commit that changes code and modifies a routed SKILL.md needed that skill; a SKILL.md the commit adds isn't counted, since there was nothing to offer yet. For each such commit, the router runs on the commit's other changed files four ways:

| method | `imports` | `importers` |
|---|---|---|
| `paths` | 0 | 0 |
| `imports` (the default) | 1 | 0 |
| `importers` | 0 | 1 |
| `both` | 1 | 1 |

Always routes count as offered. Each method reports the commits scored, recall (needed skills offered over needed), the commits with every needed skill offered, skills offered per change, and precision (needed skills offered over offered). Recall at caps 5, 6, 8 and 10 cuts the default method's list where a brief with that `skills.cap` would, in the brief's order. `--json` prints the same numbers.

The import graph is built once, from the working tree, so each commit is routed along today's imports rather than its own; a file that has since moved or gone is still matched by path. Commits come first-parent from HEAD, newest first: `--limit` (500) of them, or every one since `--since`.
