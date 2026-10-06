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
