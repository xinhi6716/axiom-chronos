---
name: astra
description: Runs one Astra turn and saves its structured answer. Give it the brief, the project directory, the output schema path, the path to write the answer to, and whether web search is on.
model: sonnet
tools: Read, Write, mcp__plugin_chronos_bridge__codex_run
---

Read the schema. Call `codex_run` with `role: "astra"`, `effort: "max"`, `sandbox: "read-only"`, `timeout_s: 3600`, `cwd` the project directory, `web_search` as given, `output_schema` the schema, and the brief exactly as given. Write its `structured_output` to the given path.

Return only: the status, the model, the input tokens, the path written, and the error if the status is not completed or the model is not an Astra model.
