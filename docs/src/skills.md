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
- **When a skill is loaded**, through Claude Code's `Skill` tool or a Gemini CLI `read_file` of a SKILL.md, it records a `use` event in `.git/fairlead/events.jsonl`. Each brief records an `offer` event with what it listed. Together they give routing a hit rate: what was offered, what was used, and what was used without being offered; `fairlead skills report` reads it.

Codex has no hook that sees a file being read, so a skill it loads isn't seen; its use is unmeasured rather than counted as a miss.

## Measuring routing: skills report

`fairlead skills report` sets the offers against the uses, per session, so a team sees whether routing offers what agents load:

```text
$ fairlead skills report --since 2026-10-01
skills report (since 2026-10-01): 4 sessions
skills: offered in 3 sessions, an offered skill used in 1 (hit rate 33%); 1 miss (used before any offer)
unmeasured: codex, whose hooks don't see a skill load; 1 session not counted as unused
1 offer with no session, not counted

  skill                    offered  used missed unused
  forms                          3     1      0      1
  money                          2     1      1      1

lessons: offered in 3 sessions; use of lessons isn't measured: there's no event for it
  lesson                   offered
  settle                         2
  tone                           1
```

Each column counts sessions:

- **offered:** a brief or an edit nudge offered the skill;
- **used:** a `use` event named it;
- **missed:** it was used before any offer of it in that session. An offer later in the session doesn't turn a miss into a hit;
- **unused:** it was offered and never used.

The hit rate is the share of sessions with a skill offered in which an offered skill was used after its offer. A brief made with no session is counted apart, since no use can be set against it.

Lessons reach the agent in the brief's text, and nothing records an agent reading one, so the report gives lessons' offers only. Codex's hooks see edits but not reads: a session whose events carry its `apply_patch` tool is listed as unmeasured, its offers count, and its skills are never counted as unused. With `.codex/hooks.json` installed, the report names Codex as unmeasured.

`--since YYYY-MM-DD` (a UTC day) and `--session ID` narrow the window, `--all` lists every row instead of the first ten, and `--json` prints the same numbers. With no events yet, the report says how they get recorded: briefs record offers, and the `PostToolUse` hook records uses.
