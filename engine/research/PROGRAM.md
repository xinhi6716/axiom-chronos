# Research program

Call the bridge `council` tool with `action: "program"`, `project` the slug, `cwd` the project directory, `brief` including the wish verbatim, and optionally `sources` as an array of relevant source paths. In creative mode, include the step 2a candidate criteria in the brief. The program replaces Astra's plan, the Critic review and the ruling in the rest of step 2a; do not run those separately. It investigates before planning, then runs the council, red team, pre-mortem, synthesis and adjudication.

Read the bounded reply: `complete` says whether the program finished, `run_id` identifies it, `stages` shows each stage and status, `estimate_line` gives the estimate, `budget` reports the cap, priced spending and unpriced tokens, `plan_final` identifies the final plan, and `note` explains the outcome. Report the note plainly. Do not treat an incomplete program as a finished plan or continue to step 3.

After a cancellation, resume by calling `program` again with the same project, cwd, brief and sources. Completed stages whose artifacts still match are reused; the program continues from the remaining work. If it cannot complete, cancel setup with `orch` `["setup", "cancel"]` and close blocked with the error.

A complete program leaves `<store>/astra/plan-final.md` for step 3, with the kept changes and coverage, and records the plan, review and rulings under `<store>/astra/`. In creative mode, apply the step 2a candidate and mode rules to the adjudicated result; if no candidate qualifies, follow the no-candidate path. Otherwise continue with the wish followed by `Carry out the changes in <store>/astra/plan-final.md.`
