---
name: wish
description: Grant a wish on the current project. The wish is a goal the session works on until it is done; the dispatcher sends Codex workers to do the work, and a Codex role rewrites their reports in plain language. Started by `/chronos:wish` or `axiom wish`.
disable-model-invocation: true
argument-hint: <wish>
---

# Wish

Wish: $ARGUMENTS

The wish is a goal. The session cannot end until you close it with the bridge `goal` tool: `done` once the wish is carried out and verified, `blocked` only when the work cannot go on without something only the user has, such as a credential, a payment or permission to delete their data.

You rule, verify and report; building goes to `chronos:dispatcher`, Astra plans to `chronos:astra`.

1. If the wish is empty, ask for it and stop. A wish marked `[creative]` enters creative mode: remove the marker and run step 2, then step 2a, then step 3 always. In creative mode `[no-architect]` is ignored; remove that marker if present. For non-creative wishes: Declare the task mode (`review`, `answer` or `change`) with the bridge `goal` tool. If it starts with `[no-architect]`, drop that marker and skip step 3. If it then starts with `[astra]`, drop that marker and do step 2a after step 2.
1a. Ask questions now only about choices that change what gets built and that the project and the wish cannot answer. If there are none, ask nothing. These are the only questions of the goal: after them, every open choice is decided by whoever meets it, written to `<store>/decisions.md` with a one-line reason, and the work goes on.
2. Name the program with a lowercase slug of the project directory's name. Call the bridge `memory_digest` tool for it. If the wish refers to earlier work, find it there.
2a. Astra thinks first:
   - Call the bridge `orch` tool with `["init"]` for the slug. Read `${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json` and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`.
   - Call `Agent` with `subagent_type: "chronos:astra"`, `project` the slug, `cwd` the project directory, `schema_path` the audit schema path, `save_to` the output path `<store>/astra/plan.json`, `web_search: false`, and `prompt` containing this brief:

     ```
     GOAL      The changes that carry out the wish below, each tied to evidence in this codebase.
     WISH      <the wish, verbatim>
     COVERAGE  In coverage.read list each path or area you actually read; in coverage.not_read list the parts of the project you did not read.
     ```

     The host runs a separate Astra planner, validates and saves the structured output, records the stage, and returns compressed path, model, usage and coverage. If it reports a failure or a model that is not an Astra model, close the goal as blocked with that error.
   - In creative mode, use the audit schema and `area: feature` to propose at most three candidates. For each, identify the existing capability, the missing link, the code location, the user-visible value, and a falsifiable acceptance check. Rank by evidence strength, then direct user value, then reuse of what exists.
   - Call the bridge `gemini_preview` tool with `project` the slug, `stage: "second_review"`, `json_schema` the review schema, and a prompt made of this paragraph, then the wish, then the plan:

     Review this change plan against its wish. Report each change that goes beyond the wish (out_of_scope), claims more than its evidence supports (overclaim), names no check that could show it wrong (unfalsifiable), or adds machinery the wish does not need (complexity). Then give a verdict.

     If it fails, give the `chronos:critic` agent that prompt, the review schema and the plan path instead. Keep the review in context until it can be written below.
   - Rule on each finding yourself: uphold or overrule it, with a one-line reason. Do not send anything back. Drop or narrow what the upheld findings name. In creative mode, choose exactly one smallest increment only if a candidate qualifies after the review and ruling; otherwise choose none. Then declare the mode after the ruling: use `change` when a candidate is chosen and `review` when none qualifies. There is no follow-up exploration.
   - After declaring the mode, write the review to `<store>/astra/review.json`, the remaining changes to `<store>/astra/plan-final.md`, and your rulings to `<store>/astra/ruling.md`. Include the unchanged coverage read and not-read lists in `plan-final.md` under `## Coverage`. In creative mode, when none qualifies, the exploration and ruling are the report and the no-change run can close done. Otherwise, from here on, the wish is the original wish followed by `Carry out the changes in <store>/astra/plan-final.md.`
3. In creative mode always; otherwise unless the wish touches a single responsibility (one function, one flag, one file), call the bridge `orch` tool with `["init"]` for the slug, and give the `chronos:architect` agent the wish, the project directory and the returned store path.
4. If creative mode chose no candidate, skip dispatch and proceed to step 5 with the exploration and ruling as the report. Otherwise hand the wish, verbatim, the absolute path of the current project directory, the slug, the module map path if there is one, and any digest lines the wish refers to, to the `chronos:dispatcher` agent. Do not plan, scope or edit the project yourself.
4a. Hand work back to `chronos:dispatcher` only when `orch` `["ready"]` lists a ready unit or a unit whose next step is `accept` or `review`. Otherwise proceed to step 5 and close the goal `done` or `blocked` with the unit states.
5. Read every file in `<store>/reports/` from the dispatcher's report, if dispatched; otherwise use the exploration and ruling as the report. In parallel:
   - Start the `chronos:memory-keeper` agent with the store path, the slug, the project directory and the wish.
   - Call the bridge `gemini_preview` tool with `project` the slug and `stage: "rewrite"`, without `cwd`. Its prompt is this line, then the wish, the dispatcher's report and those files:

     Rewrite the work reports below for the person who made the wish, in the wish's language and in plain words: what was done, how it turned out, and which choices were decided for them.

6. Show the rewrite's text. Under it, list from the dispatcher's report the changed files, the per-module acceptance results if there was a map, the full test result and whether sol ran, the choices the dispatcher decided, and what the memory keeper recorded. When step 2a ran, list Astra's coverage read and not-read lists. List every unit whose state is not `verified` with its state (`failed`, `accept-failed`, `review-failed`, or `unreviewed`). Where the rewrite's text disagrees with those facts or calls such a unit done, correct it from the facts and states. If `gemini_preview` fails, write that rewrite yourself from the same material.
7. Close the goal as `done` when its requirements are met, otherwise `blocked`, reporting the unit states.
