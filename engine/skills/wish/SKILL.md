---
name: wish
description: Grant a wish on the current project. The wish is a goal the session works on until it is done; Astra plans the wish first, the dispatcher sends Codex workers to do the work, and the core reports in plain language. Started by `/chronos:wish` or `axiom wish`.
disable-model-invocation: true
argument-hint: <wish>
---

# Wish

Wish: $ARGUMENTS

The wish is a goal. The session cannot end until you close it with the bridge `goal` tool: `done` once the wish is carried out and verified, `blocked` only when the work cannot go on without something only the user has, such as a credential, a payment or permission to delete their data.

You rule, verify and report; building goes to `chronos:dispatcher`, Astra plans to `chronos:astra`.

1. If the wish is empty, ask for it and stop. A wish marked `[creative]` enters creative mode: remove the marker and run step 2, then step 2a and its ruling. In creative mode `[no-architect]` is ignored; remove that marker if present. For non-creative wishes: Declare the task mode (`review`, `answer` or `change`) with the bridge `goal` tool. If it starts with `[no-architect]`, drop that marker and skip step 3. An old `[astra]` marker at the start of the wish is removed as plain text and changes nothing.
1a. Ask questions now only about choices that change what gets built and that the project and the wish cannot answer. If there are none, ask nothing. These are the only questions of the goal: after them, every open choice is decided by whoever meets it, written to `<store>/decisions.md` with a one-line reason, and the work goes on.
2. Name the program with a lowercase slug of the project directory's name. Call the bridge `memory_digest` tool for it. If the wish refers to earlier work, find it there.
2a. Astra thinks first: Every wish runs step 2a after step 2; there is no marker or option that skips it.
   - Call the bridge `orch` tool with `["init"]` for the slug. Read `${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json` and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`.
   - Give the `chronos:astra` agent the program slug, the project directory, the audit schema path, the output path `<store>/astra/plan.json`, web search off, and this brief:

     ```
     GOAL      The changes that carry out the wish below, each tied to evidence in this codebase.
     WISH      <the wish, verbatim>
     COVERAGE  In coverage.read list each path or area you actually read; in coverage.not_read list the parts of the project you did not read.
     ```

     If it reports a failure or a model that is not an Astra model, close the goal as blocked with that error.
   - In creative mode, use the audit schema and `area: feature` to propose at most three candidates. For each, identify the existing capability, the missing link, the code location, the user-visible value, and a falsifiable acceptance check. Rank by evidence strength, then direct user value, then reuse of what exists.
   - Give the `chronos:critic` agent the review schema, the plan path and a prompt made of this paragraph, then the wish:

     Review this change plan against its wish. Report each change that goes beyond the wish (out_of_scope), claims more than its evidence supports (overclaim), names no check that could show it wrong (unfalsifiable), or adds machinery the wish does not need (complexity). Then give a verdict.

     Write the review to `<store>/astra/review.json`.
   - Rule on each finding yourself: uphold or overrule it, with a one-line reason. Do not send anything back. Drop or narrow what the upheld findings name, write the remaining changes to `<store>/astra/plan-final.md` and your rulings to `<store>/astra/ruling.md`. Include the unchanged coverage read and not-read lists in `plan-final.md` under `## Coverage`. In creative mode, choose exactly one smallest increment only if a candidate qualifies after the review and ruling; otherwise choose none. Then declare the mode after the ruling: use `change` when a candidate is chosen and `review` when none qualifies. There is no follow-up exploration. When none qualifies, the exploration and ruling are the report and steps 5, 6 and 7 still run. Otherwise, from here on, the wish is the original wish followed by `Carry out the changes in <store>/astra/plan-final.md.`
3. In creative mode, run this step only when the ruling chose a candidate; without a candidate, only this step and the dispatcher are skipped. In other runs, run it unless the wish touches a single responsibility (one function, one flag, one file); to run it, call the bridge `orch` tool with `["init"]` for the slug, and give the `chronos:architect` agent the wish, the project directory and the returned store path.
4. If creative mode chose no candidate, skip only the architect and the dispatcher, then run step 5 (memory keeper), step 6 (report) and step 7 (close) with the exploration and ruling as the report. Otherwise hand the wish, verbatim, the absolute path of the current project directory, the slug, the module map path if there is one, and any digest lines the wish refers to, to the `chronos:dispatcher` agent. Do not plan, scope or edit the project yourself.
4a. Hand work back to `chronos:dispatcher` only when `orch` `["ready"]` lists a ready unit or a unit whose next step is `accept` or `review`. Otherwise proceed to step 5 and close the goal `done` or `blocked` with the unit states.
5. Read every file in `<store>/reports/` from the dispatcher's report, if dispatched; otherwise use the exploration and ruling as the report. Start the `chronos:memory-keeper` agent in the foreground with the store path, the slug, the project directory and the wish.
6. Write the report yourself, in the wish's language and in plain words: what was done, how it turned out, and which choices were decided for the user. When units were dispatched, list from the dispatcher's report the changed files, the per-module acceptance results if there was a map, the full test result and whether sol ran, and the choices the dispatcher decided. List what the memory keeper recorded. List Astra's coverage read and not-read lists. When units were dispatched, list every unit whose state is not `verified` with its state (`failed`, `accept-failed`, `review-failed`, or `unreviewed`); never call such a unit done.
7. Close the goal as `done` when its requirements are met, otherwise `blocked`, reporting the unit states when units were dispatched.
