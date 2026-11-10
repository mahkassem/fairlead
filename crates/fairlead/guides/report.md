# Report a mistake of Fairlead's

Use this guide when Fairlead said no and you think Fairlead is wrong, not the change: the gate failed, a hook refused an edit, a commit was stopped, a plan ran a test nothing reaches or left out one that matters, or a setup item is open that shouldn't be.

Fairlead's makers want these reports. Each one that arrives the day it happens saves a review later.

## First, check that it is Fairlead's mistake

Run what fits the refusal. Most turn out to be the change, the config or the machine.

```sh
fairlead doctor                 # the binary that ran, the versions the repository names, the hooks
fairlead setup --check          # the setup, one item to a line, with one fix each
fairlead config check           # the config's own problems
fairlead test --explain FILE    # why a test is or isn't in the plan: the chain that reaches it
fairlead guard check --list     # every finding the guard has, with its rule
```

- The gate failed on a step: run that step's command by hand. It fails the same way: the change or the machine, not Fairlead.
- A hook refused an edit: read the rule it names. The edit does what the rule forbids: fix the edit.
- A test was selected or left out: `test --explain` prints the chain. The chain is real: the plan is right.
- The binary isn't the one the repository pins: install that one, then try again.

It is Fairlead's mistake when the check shows Fairlead doing something its own output or docs say it doesn't.

## Then draft the report

```sh
fairlead report           # writes a draft inside the git folder; sends nothing
fairlead report --print   # prints the draft and writes no file
```

The draft holds the release, the platform, the config's shape as counts, the setup's states and the last refusals as numbers. Runners and checks are numbered, never named. Add three short answers: what happened, what you expected, how you checked.

## What never goes in a report

An issue is public and can't be taken back. Leave out, without exception:

- any secret: a token, password, key, cookie or connection string, even an expired one;
- any private name: a person, customer, company, repository, branch, host or domain;
- source code, file content, test output, log lines and stack traces;
- file paths that say what the project is. Write "a test file" or "the API package".

When a detail seems needed and is on this list, leave it out and say that something was left out.

## Who sends it

The project's owner. Give them the draft and say where it is; they read every line and post it at https://github.com/mahkassem/fairlead/issues/new?template=mistake.yml.

An agent posts it only when the owner said to in this session, and then only the draft and the three answers. With no such word, stop at the draft and tell the owner.

Then go on with the work: the refusal still stands until the change passes or the owner decides otherwise. A report is not a way around it.
