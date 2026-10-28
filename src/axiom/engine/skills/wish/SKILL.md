---
name: wish
description: Grant a wish on the current project. The dispatcher sends Codex workers to do the work, and Gemini rewrites their reports in plain language. Started by `axiom wish`.
disable-model-invocation: true
argument-hint: <wish>
---

# Wish

Wish: $ARGUMENTS

1. If the wish is empty, ask for it and stop. If it starts with `[no-architect]`, drop that marker and skip step 3. If it then starts with `[astra]`, drop that marker and do step 2a after step 2.
2. Name the program with a lowercase slug of the project directory's name. Call the bridge `memory_digest` tool for it. If the wish refers to earlier work, find it there.
2a. Astra thinks first:
   - Call the bridge `orch` tool with `["init"]` for the slug. Read `${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json` and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`.
   - Call the bridge `codex_run` tool with `role: "astra"`, `effort: "max"`, `sandbox: "read-only"`, `timeout_s: 3600`, `cwd` the project directory, `output_schema` the audit schema, and this brief:

     ```
     GOAL      The changes that carry out the wish below, each tied to evidence in this codebase.
     WISH      <the wish, verbatim>
     ```

     Write its `structured_output` to `<store>/astra/plan.json`. If the call fails or the model is not an Astra model, report that and stop.
   - Call the bridge `gemini_preview` tool with `model: "gemini-4-argon-high"`, `json_schema` the review schema, and a prompt made of this paragraph, then the wish, then the plan:

     Review this change plan against its wish. Report each change that goes beyond the wish (out_of_scope), claims more than its evidence supports (overclaim), names no check that could show it wrong (unfalsifiable), or adds machinery the wish does not need (complexity). Then give a verdict.

     If it fails, give the `chronos:critic` agent that prompt, the review schema and the plan path instead. Write the review to `<store>/astra/review.json`.
   - Rule on each finding yourself: uphold or overrule it, with a one-line reason. Do not send anything back. Drop or narrow what the upheld findings name, write the remaining changes to `<store>/astra/plan-final.md` and your rulings to `<store>/astra/ruling.md`. From here on, the wish is the original wish followed by `Carry out the changes in <store>/astra/plan-final.md.`
3. Unless the wish touches a single responsibility (one function, one flag, one file), call the bridge `orch` tool with `["init"]` for the slug, and give the `chronos:architect` agent the wish, the project directory and the returned store path. If it returns a question, put it to the user and stop.
4. Hand the wish, verbatim, the absolute path of the current project directory, the slug, the module map path if there is one, and any digest lines the wish refers to, to the `chronos:dispatcher` agent. Do not plan, scope or edit the project yourself.
5. If the dispatcher returns a question, put it to the user and stop.
6. Read every file in `<store>/reports/` from the dispatcher's report. In parallel:
   - Start the `chronos:memory-keeper` agent with the store path, the slug, the project directory and the wish.
   - Call the bridge `gemini_preview` tool without `cwd`. Its prompt is this line, then the wish, the dispatcher's report and those files:

     Rewrite the work reports below for the person who made the wish, in the wish's language and in plain words: what was done, how it turned out, and what they need to decide.

7. Show Gemini's text. Under it, list from the dispatcher's report the changed files, the per-module acceptance results if there was a map, the full test result and whether sol ran, the choices the dispatcher decided, and what the memory keeper recorded. Where Gemini's text disagrees with those facts, correct it from the facts. If `gemini_preview` fails, write that rewrite yourself from the same material.
