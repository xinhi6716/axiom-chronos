---
name: wish
description: Grant a wish on the current project. Astra plans, the dispatcher sends Codex workers, and the core reports. Started by `/chronos:wish` or `axiom wish`.
disable-model-invocation: true
argument-hint: <wish>
---

# Wish

Wish: $ARGUMENTS

Launch context arrives with the goal notice.

Core stage models default to planning/build/escalation/close = opus/sonnet/opus/opus; configure `models` in policy.json, then <home>/settings/models.json.

Close with the bridge `goal` tool: `done` when carried out and verified, `blocked` only when work needs something only the user has, such as a credential, payment or permission to delete their data.

You rule, verify and report; `chronos:dispatcher` builds, `chronos:astra` plans.

Reserved actions: bridge `approval` uses `request`, `status`, `execute`; only a person uses `axiom approval approve|deny`. Handle `adjudication` and `person_requests`. Classify with `goal`; resolve `interjection` via `applied` or `declined`, including `replan` and `pause_batch`. Use `orch` `policy set`.

1. If the wish is empty, ask for it and stop. Remove `[creative]` to enter creative mode. In creative mode `[no-architect]` is ignored; remove it. For non-creative wishes: Declare the task mode (`review`, `answer` or `change`) with the bridge `goal` tool. If it starts with `[no-architect]`, drop that marker and skip step 3. An old `[astra]` marker at the start of the wish is removed as plain text and changes nothing.
1a. Ask only about build choices the project and wish cannot answer. Ask no more questions: whoever meets an open choice decides it, records a one-line reason in `<store>/decisions.md` and continues.
2. Name the program with a lowercase project-directory slug. Call bridge `memory_digest`; find referenced earlier work there.
2a. Astra thinks first: Every wish runs step 2a after step 2; there is no marker or option that skips it.
   - Call the bridge `orch` tool with `["init"]` for the slug, then `["setup", "start", "--repo", <dir>]`. Read `${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json` and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`.
   - Council bullet: only when the system prompt says advanced research is on and the bridge `council` tool exists, read `${CLAUDE_PLUGIN_ROOT}/research/COUNCIL.md` and follow it; otherwise skip this bullet.
   - Give the `chronos:astra` agent the program slug, the project directory, the audit schema path, the output path `<store>/astra/plan.json`, web search off, and this brief:

     ```
     GOAL      The changes that carry out the wish below, each tied to evidence in this codebase.
     WISH      <the wish, verbatim>
     COVERAGE  In coverage.read list each path or area you actually read; in coverage.not_read list the parts of the project you did not read.
     ```

     On a planning failure, including a non-Astra model, call `orch` `["setup", "cancel"]` before closing blocked with the error.
   - In creative mode, use the audit schema and `area: feature` to propose at most three candidates. For each, identify the existing capability, the missing link, the code location, the user-visible value, and a falsifiable acceptance check. Rank by evidence strength, then direct user value, then reuse of what exists.
   - Give the `chronos:critic` agent the review schema, the plan path and a prompt made of this paragraph, then the wish:

     Review this change plan against its wish. Report each change that goes beyond the wish (out_of_scope), claims more than its evidence supports (overclaim), names no check that could show it wrong (unfalsifiable), or adds machinery the wish does not need (complexity). Then give a verdict.

     Write the review to `<store>/astra/review.json`.
   - Rule on each finding yourself: uphold or overrule it, with a one-line reason. Do not send anything back. Drop or narrow what the upheld findings name, write the remaining changes to `<store>/astra/plan-final.md`, each as a heading `### <id> <title>` with its id from `plan.json` and your rulings to `<store>/astra/ruling.md`. Include the unchanged coverage read and not-read lists in `plan-final.md` under `## Coverage`, after the changes. In creative mode, choose exactly one smallest increment only if a candidate qualifies after the review and ruling; otherwise choose none. Then declare the mode after the ruling: use `change` when a candidate is chosen and `review` when none qualifies. There is no follow-up exploration. Except when creative mode chose none, from here on, the wish is the original wish followed by `Carry out the changes in <store>/astra/plan-final.md.`
3. In creative mode, run this step only when the ruling chose a candidate. In other runs, skip it only for one function, flag or file. Call `orch` `["init"]` for the slug and give `chronos:architect` the wish, project directory and returned store path. On failure, call `orch` `["setup", "cancel"]` before closing blocked.
3a. After the ruling, or after the map when the architect runs, call `orch` `["setup", "keep", <ids>...]` for setup steps the final plan needs, or `["setup", "cancel"]` if none.
4. If creative mode chose no candidate, skip only the architect and the dispatcher, then run step 5 (memory keeper), step 6 (report) and step 7 (close) with the exploration and ruling as the report. Otherwise hand the wish, verbatim, the absolute path of the current project directory, the slug, the module map path if there is one, and referenced digest lines to `chronos:dispatcher`; if `memory_digest` is empty, say "no memory". Dispatchers report outcomes; the module-done outcome is the module report. With a map, loop on `orch` `["schedule", "next", "--cursor", <n>]`, starting at 0 and using each reply's `cursor` next. For each module in `start`, start a fresh `chronos:dispatcher` with `run_in_background: true`, naming its module. On `stuck`, decide: waive, then call `orch` `["schedule", "claim", <module>]`, or close blocked. On `done`, start one last fresh foreground dispatcher for the full test. Without a map, keep the dispatcher in the foreground. Never poll, sleep or watch files while an agent runs. Do not plan, scope or edit the project yourself.
4a. After a `fix_rounds` report, waive findings with `orch` `["waive", <unit>, <n>, "--reason", <text>]` or close the goal as `blocked`. Hand work back to `chronos:dispatcher` only when `orch` `["ready"]` lists a ready unit or a unit whose next step is `accept` or `review`, or the report shows `handoff pending`; the batch pilot and launch of step 7 are the one exception. Otherwise proceed to step 5 and close the goal `done` or `blocked` with the unit states.
5. If units were dispatched, call the bridge `orch` tool with `["report", "--repo", <project directory>]`: its digest of unit states, acceptance, the full test, sol, changed files, findings, decisions and Astra's coverage is the report. Never read `<store>/reports/` file by file; a section the digest cuts names the file holding the rest. Otherwise use the exploration and ruling as the report. Start the `chronos:memory-keeper` agent in the foreground with the store path, the slug, the project directory and the wish.
6. In one message make two parallel calls: the step 7 `goal` close and `chronos:narrator`. Do not reprint its text. Give `chronos:narrator` the `orch` `["report"]` digest to fetch itself: pass only the slug, project directory, store path, wish's language and any ruling not already in `<store>/astra/ruling.md`.  End the report with one `Provenance:` line that copies every field of the last entry of `provenance` in the latest `goal` tool result, `unknown` values included; once both calls return, that line is your whole reply. The rulings stay yours; step 7 closes the goal.
7. Close the goal in the same message as step 6's narrator call (a `waiting` close has none), as `done` when its requirements are met, otherwise `blocked`, reporting the unit states when units were dispatched. When what remains is a long batch of the project's own programs, read `${CLAUDE_PLUGIN_ROOT}/skills/wish/BATCH.md` and follow it.
