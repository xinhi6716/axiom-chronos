---
name: wish
description: Run an Axiom Chronos wish or resume a wish session in the user's project.
---

# Wish

Prefer the configured native MCP `session` tool. For a new wish call it with `action: "begin"`, `project` as the absolute project directory, `skill: "wish"`, and `text` containing the user's wish and applicable `[creative]`, `[no-architect]` and `[astra]` intent. For an existing session use `action: "resume"` and `session` with its id. Preserve the user's project, flags, text and intent; do not silently discard an option the native schema cannot represent (such as `--max-turns`). Report the limitation or use a compatible CLI path when available.

Keep the same MCP connection open. The response supplies `workflow` and Host `tools`; the current caller is the sole core. Follow that workflow and invoke its returned tools through `session` with `action: "call"`, `tool` and `arguments` on that connection. Let the engine workflow own role dispatch; do not start another core or native collaboration subagents. Calls are synchronous: await the dispatcher response and its bound notices rather than polling status.

If native session is unavailable, invoke the installed `axiom wish` in the user's project, forwarding flags and text in their original order and argument boundaries with safe argument quoting. `--resume <session-id>` accepts only optional `--max-turns`. On the new binary CLI output is attachment/bootstrap information, not completed work; continue through a live native session rather than treating exit code 0 as success. Report actual evidence-gated goal outcome and any blocker, not model completion.

The Rust role authority is mandatory: core calls only dispatcher and the workflow intent verdict/submission endpoint. Dispatcher routes Astra MAX research, core pass/reject, Astra HIGH planning, Terra implementation, Sol assembly, and separate Astra HIGH testing. Never let core take over engineering or bypass a rejected phase. Await the live call rather than using GOAL continuation or polling.

Once the dispatcher command is issued, core has no further work until that same command returns an authority boundary. Do not read files/status, redispatch, or reason about elapsed time while it is pending. Rust retains stage continuation and ordinary test repairs in the dispatcher subtree and holds a per-session caller command lock. Use the host's longest supported blocking await for the pending call (the user's requested wait window is 1200 seconds); no timer-driven status checks or periodic no-change messages. If the host forces intermediate re-entry, do not misrepresent it as a Chronos wakeup. Only intent_review, ready_submit or a genuine terminal failure requires core action.
