---
name: research
description: Run an Axiom Chronos research question in the user's project.
---

# Research

Prefer the configured native MCP `session` tool. For a new question call it with `action: "begin"`, `project` as the absolute project directory, `skill: "research"`, and the user's question as `text`. To resume, use `action: "resume"` and `session` with its id. Preserve project, flags and user intent; do not silently discard an option the native schema cannot represent (such as `--max-turns`). Report the limitation or use a compatible CLI path when available.

Keep the same MCP connection open. The response supplies `workflow` and Host `tools`; the current caller is the sole core. Follow that workflow and invoke its returned tools through `session` with `action: "call"`, `tool` and `arguments` on that connection. Let the engine workflow own role dispatch; do not start another core or native collaboration subagents. Research is read-only and plan-only, without engineering gates. Await synchronous calls rather than polling status.

If native session is unavailable, bootstrap with `axiom research`, retain its session id, and use the durable `axiom session-call ABSOLUTE_REQUEST_FILE` transport documented in `../wish/SKILL.md`. Await the same host process; do not host long bridge calls inside node_repl. Replaying the same immutable request id retrieves its persisted result without another model call. Bootstrap exit code zero is not completed research; follow the returned workflow.

The Rust role authority is mandatory: core calls only dispatcher and the workflow intent verdict/submission endpoint. Dispatcher routes Astra MAX research, core pass/reject, Astra HIGH planning, Terra implementation, Sol assembly, and separate Astra HIGH testing. Never let core take over engineering or bypass a rejected phase. Await the live call rather than using GOAL continuation or polling.
