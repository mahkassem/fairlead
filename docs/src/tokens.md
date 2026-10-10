# Tokens and cost

*Since 0.10.0*, Fairlead records how many tokens each agent session used, and what it cost when the source says. Only numbers are read.

```sh
fairlead tokens report                 # the newest totals of each session and source
fairlead tokens report --session ID --json
fairlead tokens import result.json     # a headless run: claude -p --output-format json > result.json
```

## Sources

| Source | How it's read | Cost |
|---|---|---|
| `transcript` | The Stop hook reads the session's transcript, from the `transcript_path` Claude Code passes it. | yes |
| `headless` | `fairlead tokens import` reads the JSON `claude -p --output-format json` prints. | yes |

From a transcript, Fairlead reads only the `cost-state` lines Claude Code writes. These are Claude Code's own running totals for each process: tokens per model and the cost, counting subagents and its background calls. The last line of each process is summed, so a resumed session adds up.

A transcript without `cost-state` lines, from an older Claude Code, is counted from the usage on each assistant message instead. Each message is counted once, though a streamed reply repeats its usage on several lines, and the subagent transcripts beside the session's are included. Those totals are marked `estimate`: a message's usage can be written before its reply finishes streaming, so output can be undercounted, and background calls aren't there at all. No price table ships with Fairlead, so an estimate has no cost.

Each Stop records the session's totals so far, and a later record of the same session and source replaces the earlier one. Codex and Gemini CLI sessions aren't counted yet.

## What reaches the log

A `tokens` event holds the session id, the source and the five numbers (input, output, cache read, cache write, cost), and nothing else from the transcript. The transcript's text is never parsed. Only the `cost-state` lines are, or the assistant lines' usage in the fallback. `guard.events = "off"` stops the records, as it does for the hook's.
