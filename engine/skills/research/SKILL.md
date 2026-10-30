---
name: research
description: Turn a research question into a ruled research plan. Astra drafts it at maximum effort, a distinct Codex role reviews it for overreach, and you make the final ruling. Started by `axiom research`.
disable-model-invocation: true
argument-hint: <question>
---

# Research

Question: $ARGUMENTS

1. If the question is empty, ask for it and stop.
2. Name the program with a lowercase slug of the project directory's name and call the bridge `orch` tool with `["init"]` for it. Research files go in `<store>/research/`.
3. Read `${CLAUDE_PLUGIN_ROOT}/research/plan.schema.json` and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`.
4. Call `Agent` with `subagent_type: "chronos:astra"`, `project` the slug, `cwd` the project directory, `schema_path` the plan schema path, `save_to` the output path `<store>/research/plan.json`, `web_search: true`, and `prompt` containing this brief:

   ```
   GOAL      A research plan for the question below, with every field of the output schema filled.
   QUESTION  <the question, verbatim>
   ```

   The host runs a separate Astra planner, validates and saves the structured output, records the stage, and returns compressed path, model, usage and coverage. If it reports a failure or a model that is not an Astra model, report that and stop.
5. Call the bridge `gemini_preview` tool with `project` the slug, `stage: "second_review"`, `json_schema` the review schema, and a prompt made of this paragraph, then the question, then the plan:

   Review this research plan against its question. Report each item that goes beyond the question (out_of_scope), claims more than its evidence supports (overclaim), cannot be shown wrong by any test the plan lists (unfalsifiable), or adds machinery the question does not need (complexity). Then give a verdict.

   If it fails, give the `chronos:critic` agent that prompt, the review schema and the plan path instead. Write the review to `<store>/research/review.json`.
6. Rule on each finding yourself: uphold it or overrule it, with a one-line reason grounded in the question and the plan. Do not send anything back to the planner or reviewer. Apply the upheld findings to the plan by removing or narrowing what they name, and write the result to `<store>/research/plan-final.md`, and your rulings to `<store>/research/ruling.md`.
7. Show the final plan in the question's language. Under it, list the review verdict, each finding with your ruling and reason, and the Astra model and its input tokens.
