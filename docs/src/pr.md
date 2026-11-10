# Pull request rules

*Since 0.18.0:* what a change has to meet before anyone reviews it, such as a
branch name, a ticket key, the base it targets or a release note, declared
once in `[pr]` and checked in three places: `fairlead done`, the pre-push
hook, and `fairlead pr check` in CI. A check that only CI runs fails after
the push, often more than once; the same rules before the push fail where
they're cheap to fix.

```toml
[pr]
base = "main"                         # the branch changes target
up_to_date = true                     # the default: the branch contains base's head
branch = '^(feat|fix|chore)/[a-z0-9-]+$'
title = '^(feat|fix|chore)(\(.+\))?: .+'
ticket = { pattern = '[A-Z]+-[0-9]+', in = ["branch", "title"], any = true }
body_headings = ["Summary", "Testing"]
labels = { forbid = ["do-not-merge"], require_any = ["bug", "feature", "chore"] }

[[pr.files]]
path = ["changes/*.md"]               # the change adds or edits one of these
when = ["src/**"]                     # only when it touches these; any change when left out
unless_label = "no-change-note"       # a label that excuses it
why = "every user-visible change has a note"
```

| Rule | Key | Fails when |
| --- | --- | --- |
| `pr-base` | `base` | the pull request targets another branch |
| `pr-stale` | `base`, `up_to_date` | the branch doesn't contain the base's head: it's behind, or was cut from another branch |
| `pr-branch` | `branch` | the branch name doesn't match the regex |
| `pr-title` | `title` | the title doesn't match the regex |
| `pr-ticket` | `ticket` | the key isn't in each place `in` names, or in any of them with `any = true`; `in` is `["title"]` when left out |
| `pr-body` | `body_headings` | the description has no Markdown heading with that text, at any level |
| `pr-labels` | `labels` | a `require` label is missing, none of `require_any` is set, or a `forbid` label is set |
| `pr-file` | `[[pr.files]]` | the change adds or edits no file matching `path`, when it touches `when` and isn't labelled `unless_label` |

Each failure prints the rule, what was found and the fix, such as
`pr-branch: the branch name tidy-up doesn't match ^(feat|fix)/` and how to
rename it. `config check` checks every regex and glob.

## Where it runs

```sh
fairlead pr check          # in CI: the pull request event; elsewhere: git, and gh when signed in
fairlead pr check --local  # ignore a CI event and check what this machine knows
```

- **`fairlead done`** ends with `pr check --local` when `[pr]` is set.
- **Before a push**, `hooks install` adds `fairlead-push` to lefthook's
  `pre-push`, which runs `fairlead guard check --push`: the
  [migration numbers](guard.md#migrations) and then these rules, with the
  base fetched first (3 seconds, then the local ref with a warning).
  *Since 0.18.0:* the entry runs `fairlead done --pre-push`, which runs the
  change's plan and then `guard check --push` ([Before a push](done.md#before-a-push)).
- **In CI**, `fairlead pr check` reads the pull request from the event file
  GitHub Actions names in `GITHUB_EVENT_PATH`: its title, body, labels, base
  and head. Run it on `pull_request` events, with the history it needs:

```yaml
on:
  pull_request:
    types: [opened, edited, synchronize, reopened, labeled, unlabeled]
jobs:
  pr:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: mahkassem/fairlead@v0.20.1
        with:
          command: pr check
```

`edited`, `labeled` and `unlabeled` rerun it when the title, body or labels
change, which no push does.

## Claims and receipts

*Since 0.18.0:* "verified" in a pull request body should mean a gate passed on
the code being reviewed. `fairlead receipt --pr` prints the
[receipt](receipt.md) as a block for the body, marked with the tree it's for
and the gate's state:

```sh
fairlead done && fairlead receipt --pr    # paste the block into the pull request body
```

`pr check` reads the body for claims: `verified`, `tested`, `tests pass`
(`passed`, `passing`, `are green`), `checks pass` and the like, `all tests
pass`, `all green` and `works as expected`, case aside. A claim with `not`,
`no`, `never`, `without` or a `n't` word among the three words before it
doesn't count, nor do code blocks, HTML comments, receipt blocks and unticked
task items (`- [ ]`). A body with a claim needs a receipt for the head's tree
whose gate passed:

- **In CI**, the receipt is a block in the body, or a file given with
  `pr check --receipt FILE` (a `receipt --out x.json` file, or one holding
  blocks), whose tree is the head commit's tree; otherwise the check fails as
  `pr-claims`. A receipt for an earlier tree, before the last push, doesn't
  count.
- **On a machine**, a passing `fairlead done` for the tree as it stands
  counts too. Until one has passed, the rule is not checked here, since
  `done` runs `pr check` before it records its own outcome.

```text
pr: 1 rule(s) failed:
  pr-claims: the body says "verified", and no receipt for the head's tree passed
    fix: run `fairlead done` on the head and put `fairlead receipt --pr` in the body, or take the claim out
```

The words are a short, generic list on purpose, and anyone can type a block
by hand: this keeps an honest agent's claims tied to a run, not a liar out.
`[pr] claims = false` turns it off. [`ci report`](ci.md#fairlead-ci-report---plan-path)
shows the body's receipt in its summary and comment.

## What each place can tell

A rule whose facts a place doesn't have is reported as `not checked here`,
never as passed. On a machine, the branch, the base's history and the
changed files come from git; the title, body and labels come from `gh pr view`
when a pull request is open and `gh` is signed in, and are skipped after 5
seconds. Before the pull request exists, a `[[pr.files]]` rule that a label
could excuse is left unchecked when the file is missing, since the label may
come later.

In CI nothing may be left unchecked: a rule CI can't check fails as
`pr-history`, with the fix `fetch-depth: 0`, since a shallow checkout lacks
the commits that show whether the branch contains the base and which files
it changed. A commit missing from the checkout is fetched by its id first.

`fairlead next` lists the rules under the pull request's step, so an agent
sees them before it opens one. In the [guard-only profile](install.md#the-guard-only)
`done` runs only the guard and these rules.
