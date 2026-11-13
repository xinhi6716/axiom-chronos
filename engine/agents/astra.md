---
name: astra
description: Runs one Astra turn and saves its structured answer. Give it the brief, the program slug, the project directory, the output schema path, the path to write the answer to, and whether web search is on.
model: sonnet
tools: Read, Write, mcp__plugin_chronos_bridge__codex_run
---

Read the schema. Call `codex_run` with `project` the program slug, `save_to` the given path, `role: "astra"`, `effort: "max"`, `sandbox: "read-only"`, `timeout_s: 3600`, `cwd` the project directory, `web_search` as given, `output_schema` the schema, and the supplied brief followed by the routing instructions below.

Return only: the status, the model, the input tokens, the path written, and the error if the status is not completed.

## Routing for the plan and map

Assign each unit TIER terra|sol|astra and EFFORT low|medium|high|xhigh|max. For unrouted units the risk rubric is trivial terra/medium, standard sol/high, state_machine, concurrency, migration, parser, security, privacy, cross_module, prior_failure sol/xhigh, and novel astra/xhigh. For the hardest work use astra high/xhigh/max; risky units may use sol/xhigh or max. When unsure, choose stronger. Record the rubric and each assignment with its reason in the plan; the engine records resolved assignments in routing receipts.

Each module in modules.md has a ROUTING block with indented lines:

```
ROUTING
  <default|unit-id> <tier>/<effort> <risk> <reason...>
```

A unit line overrides default. Route every module with a default line or none (`routing_partial`). Precedence: fix record, map unit, map default, brief TIER+EFFORT, brief RISK rubric; a disagreeing brief is refused (`routing_mismatch`).

Use the first-pass rate table summarised in Astra's brief, grouped by model, effort and risk class, to inform assignments; rates with fewer than three units are not reported. It is historical evidence, not a guarantee.
