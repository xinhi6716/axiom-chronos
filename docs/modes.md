# Modes

`axiom modes` lists the six modes. The Windows desktop app has Chat, Quick task, Plaza and Routine pages over the engine APIs below; chat and plaza have no CLI command. `axiom::modes::api::ModesApi` opens sessions and reads sessions, transcripts, bounded context, and runs.

## Chat

`axiom::modes::chat::Chat` opens or resumes a session, accepts messages through a provider and read-only tools, and can launch a small action or promote the conversation. The desktop Chat page uses it.

## Quick task

`axiom do [--project DIR] --scope PATH... --verify CMD [--review] [--model M] "<task>"` runs one scoped task. Repeat `--scope` or use comma-separated paths. Project defaults to the current directory; review is opt-in. The engine API is `axiom::modes::quick::{Brief, QuickConfig, run_quick}`.

## Plaza

`axiom::modes::plaza` opens or resumes a read-only discussion room. Its router selects at most three distinct members and records whom it chose and why; routing never grants permissions. `plaza::access` enforces per-member call and optional cost budgets. `plaza::pins` records selected messages as pins and can promote a thread. The desktop Plaza page uses it.

## Wish

`axiom wish` starts the wish workflow. Chat or plaza can promote to quick or wish through `axiom::modes::promotion::promote` and a launcher: the child run retains its parent and carries a conversation snapshot and source references forward.

## Research

`axiom research` starts the research workflow for investigation. Mode sessions and their transcript and run history are accessible through `axiom::modes::api::ModesApi`.

## Routine

`axiom::modes::routine::{load, tick, fire_event}` loads opt-in configurations and handles cron ticks or events with a launcher. Routines are off unless `<home>/modes/routines.json` exists and an entry has `"enabled": true`. For example:

```json
[
  {
    "id": "daily-check",
    "enabled": true,
    "trigger": { "type": "cron", "expr": "0 9 * * *" },
    "launch": "quick",
    "goal": "Update the check",
    "scope": ["src/check.py"],
    "verify": "python -m unittest",
    "project": "/path/to/project"
  }
]
```

An event trigger instead uses `{"type":"event","kind":"ci_failure","repo":"owner/repo"}`; `repo` may be null. Launch is `quick` or `wish`; quick requires verification.

## Receipts

`axiom::modes::receipts` stores run receipts with elapsed time and cost. If usage or pricing is unavailable, the rendered receipt says `cost unknown`. CLI receipt output is text, including the quick-task store path.
