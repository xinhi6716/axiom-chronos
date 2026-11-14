---
name: wish
description: Grant a wish on the current project. Astra plans, the dispatcher sends Codex workers, and the core reports. Started by `/chronos:wish` or `axiom wish`.
disable-model-invocation: true
argument-hint: <wish>
---

# Wish

Wish: $ARGUMENTS

Launch context: goal notice.

Core stage models: planning/build/escalation/close = opus/sonnet/opus/opus; configure `models` in policy.json, then <home>/settings/models.json.

Close with bridge `goal`: `done` when carried out and verified, `blocked` only when work needs something only the user has, such as credentials, payment or permission to delete their data.

You rule, verify and report; `chronos:dispatcher` builds, `chronos:astra` plans. The dispatcher and core never choose worker model or effort; routing does and terra is the floor.

Reserved: bridge `approval` uses `request`, `status`, `execute`; only a person uses `axiom approval approve|deny`. Handle `adjudication` and `person_requests`. Classify with `goal`; resolve `interjection` via `applied` or `declined`, including `replan` and `pause_batch`. Use `orch` `policy set`.

1. If empty, ask and stop. Remove `[creative]` to enter creative mode. In creative mode `[no-architect]` is ignored; remove it. Otherwise: Declare the task mode (`review`, `answer` or `change`) with the bridge `goal` tool. If it starts with `[no-architect]`, drop it and skip step 3. An old `[astra]` marker at the start of the wish is removed as plain text and changes nothing.
1a. Ask only about build choices the project and wish cannot answer. Whoever meets a choice decides, records a one-line reason in `<store>/decisions.md`, and continues without questions.
2. Name the program with a lowercase project-directory slug. Call `memory_digest`; find referenced work.
2a. Every wish runs step 2a after step 2; there is no marker or option that skips it.
   - Call bridge `orch` with `["init"]` for the slug, then `["setup", "start", "--repo", <dir>]`. Read `${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json` and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`.
   - Council bullet: if the system prompt says advanced research is on and bridge `council` exists, read `${CLAUDE_PLUGIN_ROOT}/research/COUNCIL.md` and follow it; otherwise skip.
   - Give the `chronos:astra` agent slug, project directory, audit schema path, output path `<store>/astra/plan.json`, web search off, and this brief:

     ```
     GOAL      Changes carrying out the wish below, each tied to codebase evidence.
     WISH      <the wish, verbatim>
     COVERAGE  In coverage.read list each path or area read; in coverage.not_read list the parts not read.
     ```

     On planning failure, including non-Astra, call `orch` `["setup", "cancel"]`, then close blocked with the error.
   - In creative mode use the audit schema and `area: feature`; propose at most three candidates. For each identify the existing capability, the missing link, the code location, the user-visible value, and a falsifiable acceptance check. Rank by evidence strength, then direct user value, then reuse of what exists.
   - Give the `chronos:critic` agent the review schema, the plan path and this paragraph then the wish:

     Review this plan against its wish. Report changes beyond the wish (out_of_scope), claims beyond evidence (overclaim), changes with no falsifying check (unfalsifiable), or unnecessary machinery (complexity). Give a verdict.

     Write review to `<store>/astra/review.json`.
   - Rule on each finding: uphold or overrule with a one-line reason. Send nothing back. Drop or narrow upheld findings; write remaining changes to `<store>/astra/plan-final.md`, each as a heading `### <id> <title>` with id from `plan.json` and rulings to `<store>/astra/ruling.md`. Include unchanged coverage read and not-read lists under `## Coverage` in `plan-final.md` after changes. In creative mode choose one smallest qualifying increment after review and ruling, or none. Declare `change` if chosen, `review` if none; no follow-up exploration. Unless creative mode chose none, append `Carry out the changes in <store>/astra/plan-final.md.` to the wish.
3. In creative mode run this step only when the ruling chose a candidate. Otherwise skip only for one function, flag or file. Call `orch` `["init"]` for the slug and give `chronos:architect` the wish, project directory and returned store path. On failure, call `orch` `["setup", "cancel"]`, then close blocked.
3a. After ruling or architect map, call `orch` `["setup", "keep", <ids>...]` for final-plan setup steps, or `["setup", "cancel"]` if none.
4. If creative mode chose none, skip only architect and dispatcher; run steps 5 (memory keeper), 6 (report) and 7 (close) with exploration and ruling as the report. Otherwise give `chronos:dispatcher` the verbatim wish, absolute project directory, slug, optional map path and referenced digest lines; if `memory_digest` is empty, say "no memory". Dispatchers report outcomes; the module-done outcome is the module report. With a map, call `orch` `["schedule", "next", "--cursor", <n>]` starting at 0, using reply cursor. For each claimed module in `start`, start a fresh `chronos:dispatcher` naming its module with `run_in_background: true`; call next after each start and each completion notice so ready modules start while others run. That value is the JSON boolean, never a string; per unit, record the module ACCEPT, then the final review, and re-review if a re-run ACCEPT saw changed files. On `stuck`, waive and call `orch` `["schedule", "claim", <module>]`, or close blocked. On `done`, start a fresh dispatcher for full test in the foreground. Without a map, run the dispatcher in the foreground. Bracket Claude-side module <m> work with `orch` `["activity","start"|"finish","--module",<m>,"--actor","core","--phase","decide"]`. Never poll, sleep or watch files while an agent runs. Do not plan, scope or edit the project yourself.
4a. After a `fix_rounds` report, waive findings with `orch` `["waive", <unit>, <n>, "--reason", <text>]` or close the goal as `blocked`. Hand work back to `chronos:dispatcher` only when `orch` `["ready"]` lists a ready unit or a unit whose next step is `accept` or `review`, or the report shows `handoff pending`; the batch pilot and launch of step 7 are the one exception. Otherwise proceed to step 5 and close `done` or `blocked` with unit states.
5. If units were dispatched, call `orch` `["report", "--repo", <project directory>]`: its digest of unit states, acceptance, full test, sol, changed files, findings, decisions and Astra's coverage is the report. Never read `<store>/reports/` file by file; a cut section names its file. Otherwise report exploration and ruling. Start the `chronos:memory-keeper` agent in the foreground with store path, slug, project directory and wish.
6. In one message make two parallel calls: step 7 `goal` close and `chronos:narrator`. Do not reprint its text. Give `chronos:narrator` the `orch` `["report"]` digest to fetch itself: pass only slug, project directory, store path, wish's language and any ruling not already in `<store>/astra/ruling.md`. End the report with one `Provenance:` line copying every field of the last `provenance` entry in the latest `goal` result, including `unknown` values; once both return, that line is your whole reply. The rulings stay yours; step 7 closes the goal.
7. Close in step 6's narrator message (a `waiting` close has none): `done` when requirements are met, otherwise `blocked`; report unit states when dispatched. When only a long batch remains, read `${CLAUDE_PLUGIN_ROOT}/skills/wish/BATCH.md` and follow it.
