---
name: wish
description: Run a Chronos wish through the configured Rust bridge.
---

# Wish

Call the bridge `dispatch` tool with `project`, absolute `repo_root`, a unique `request_id`, and `action: {"type":"start","intent":"..."}`. One run implements ONE round. For a multi-round task, keep the remaining rounds as a short TODO; send only the current round's deliverables and inherited constraints as intent. Each round must reach `ready_to_commit`, then the core performs the user-required Git/release steps and confirms them before starting the next run. Commit/push/install are post-verification core actions, never pre-commit product checks.

Await the same call to its boundary; if the tool wrapper yields, await that call's completion. Do not poll, read status/source just to fill the wait, start another core, or inject a continuation. Keep the request id and identical input for replay after transport loss. A lost host connection does not guarantee that the process survives.

At `needs_intent_review`, read only `research.path`, assess compatibility with user intent, and call `dispatch` with the same project/root, a new request id and `action: {"type":"decide","run_id":"<returned run_id>","research_digest":"<research.digest>","pass":true,"reason":"<short intent judgment>"}`. Use false with the shortest conflict reason to request revised research. Do not guess the digest from artifact order or pass automatically.

For an existing run use `status`, `resume`, or `cancel` as appropriate. Unknown writes require reconciliation, not automatic redispatch. A blocked run is not permission to switch to `codex_run`, `orch` engineering, native agents, or core implementation. Preserve the failure evidence and use the managed recovery path; if it cannot recover, report the blocker. Report `ready_to_commit` as ready, not released. Exit zero means the request reached a boundary, not that work or installation completed.

For a known planning failure before any frozen plan or writes, explain the corrected prerequisite and resume with `recovery: {"retry_planning":{"reason":"<specific repair/context>"}}`. Rust requires all prior attempts terminal, preserves research and failure evidence, and permits at most two planning retries. It never unfreezes an installed plan or retries unknown writes. Do not blindly repeat an unchanged failure.

Independent acceptance preparation is built in: the planner requests a separate Astra HIGH tester to define and seal private executable checks before baseline/engineering. Do not require the user or core to create those test materials, or label public regression tests as held-out acceptance.
