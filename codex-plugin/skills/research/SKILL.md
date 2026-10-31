---
name: research
description: Run an Axiom Chronos research question in the user's project.
---

# Research

Prefer the configured native MCP `session` tool. For a new question call it with `action: "begin"`, `project` as the absolute project directory, `skill: "research"`, and the user's question as `text`. To resume, use `action: "resume"` and `session` with its id. Preserve project, flags and user intent; do not silently discard an option the native schema cannot represent (such as `--max-turns`). Report the limitation or use a compatible CLI path when available.

Keep the same MCP connection open. The response supplies `workflow` and Host `tools`; the current caller is the sole core. Follow that workflow and invoke its returned tools through `session` with `action: "call"`, `tool` and `arguments` on that connection. Let the engine workflow own role dispatch; do not start another core or native collaboration subagents. Research is read-only and plan-only, without engineering gates. Await synchronous calls rather than polling status.

If native session is unavailable, invoke the installed `axiom research` in the user's project, forwarding flags and text in their original order and argument boundaries with safe argument quoting. On the new binary CLI output is attachment/bootstrap information, not completed research; continue through a live native session rather than treating exit code 0 as success. Report the actual goal outcome and artifacts or failure.

The Rust role authority is mandatory: core calls only dispatcher and the workflow intent verdict/submission endpoint. Dispatcher routes Astra MAX research, core pass/reject, Astra HIGH planning, Terra implementation, Sol assembly, and separate Astra HIGH testing. Never let core take over engineering or bypass a rejected phase. Await the live call rather than using GOAL continuation or polling.
