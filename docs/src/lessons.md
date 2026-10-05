# Lessons

A lesson is one thing a project learned the hard way, kept as a small file a person can review in a pull request. `fairlead brief` offers the lessons a change reaches, the way the plan picks the tests it reaches. *Since 0.8.0.*

```bash
fairlead learn --title "Leave settles at sign-off" --path "src/leave/**" \
  --evidence https://github.com/acme/app/pull/812 \
  --source person --confirmed-by a-reviewer \
  --body "A decision that costs money happens at the final stage, and nowhere else."
fairlead lessons list           # every lesson and its scope
fairlead lessons review         # the ones past their review date
fairlead lessons check          # fail on a file that can't be offered, for CI
```

## A lesson file

Each lesson is a file in `.fairlead/lessons/` (`memory.dir`), named after its `id`:

```markdown
---
id: leave-settles-at-sign-off
title: "Leave settles at sign-off"
paths: ["src/leave/**", "src/domain/settlement.ts"]
added: 2026-09-12
review_by: 2026-12-11
evidence: ["https://github.com/acme/app/pull/812"]
check: "guard:migrations"
source: person
confirmed_by: "a-reviewer"
---
A decision that costs money happens at the final stage, and nowhere else.
```

- **Scope:** `paths` (globs), `modules` (module names), or `always: true`. At least one is needed.
- **Evidence:** at least one link to where it was learned: a pull request, a CI run, a review.
- **Source:** where it came from. Something learned has to be confirmed or seen to fail:
  - `person`: a person confirmed it, and `confirmed_by` names them;
  - `mistake`: an obvious mistake taught it, and the evidence is the failure, such as a CI run, a receipt or a failing check;
  - `imported`: brought in from a document the team already kept.
- **check** (optional): the rule or test that enforces it. An enforced lesson is cheap to trust, so it's listed first.
- **The body:** at most `memory.max_lines` lines (12). Anything longer is a doc, and the lesson links to it.
- **Review:** `review_by` is `added` plus `memory.review_days` (90). Past it, the lesson is still offered, marked due, and `fairlead lessons review` lists it. Nothing fails because a lesson is due.

A file that breaks any of these, or that looks like it carries a secret or personal data (a key, a token, a password, an email address, a card number), isn't offered. The brief names it as `[bad-lesson] path: reason`, `fairlead doctor` lists it, and `fairlead lessons check` exits 1 on it. A finding names the kind and the line, never the value.

## How a change picks its lessons

A lesson is offered when its scope covers, in this order:

1. **named:** a path the change names;
2. **used:** a file a named path imports, one hop away. The walk goes along imports, not importers: a file needs the contracts it uses, and nothing imports a test, so a test's lessons are only reachable forward. It stops at a [`graph.barrier`](graph.md#barriers), so a barrel that re-exports a package doesn't pull in everything behind it;
3. **always:** an `always` lesson, for every change;
4. **fallback:** when the plan runs everything, such as for a lockfile, every lesson is offered and marked as the fallback.

Within each group, enforced lessons come first, then the newest. The brief lists `memory.cap` (5) of them, then how many more and `fairlead brief --all`.

## Writing one: `fairlead learn`

`fairlead learn` writes one lesson from flags, reading the body from `--body` or stdin. It refuses a lesson with no scope, no evidence, no source, `--source person` without `--confirmed-by`, a body over the cap, an id that's already taken, or text the secrets check flags.

An agent can call it. The file lands in the working tree only, never committed, so a person reviews it in the pull request it rides in. With `memory.learn = "ask"`, `learn` prints the file instead of writing it, for a team that wants a person to save each one.
