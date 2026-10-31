---
name: wish
description: Carry out user intent through the Rust-enforced Chronos role workflow.
---

# Wish

Wish: $ARGUMENTS

The current caller is the root host; do not start another core model. Core owns no technical design, rulings, implementation, assembly. Read-only is not an inability to dispatch.

The current caller is the sole core. Core communicates with the user and relays the user's intent; it does not research, design, split work, implement, test, build or recover engineering operations.

1. Declare the user-requested mode through `goal`, then call `Agent` with `subagent_type: "chronos:dispatcher"`, the project directory and the user's intent unchanged.
2. Dispatcher calls independent `subagent_type: "chronos:astra"` (Astra MAX). Rust returns an `intent_review` boundary containing a bounded intent summary and research revision.
3. Core checks ONLY whether the proposal conflicts with user intent. Call `workflow` action `verdict`, current `revision`, and `decision: "pass"`; or `decision: "reject"` with the shortest conflict reason. Do not add technical designs or instructions. Rejection returns to dispatcher/research. Pass releases the research context and unlocks Astra HIGH planning.
4. Call dispatcher again to relay the verdict and continue. Dispatcher owns Astra HIGH planning, Terra modules, Sol assembly, Astra HIGH tests and ordinary repairs. Await the synchronous call; never poll, use GOAL auto-continuation, inspect worker logs or take over engineering while waiting.
5. Only the Rust `ready_submit` boundary permits core to call `workflow` action `submit`. Then close the completed session with `goal` status `done`. Submission never substitutes for missing verification. Report the actual result, not model completion.

Keep role contexts separate. The researcher's technical artifact remains available to the planner, but its live context is released on approval. Astra HIGH planning and Astra HIGH testing are distinct contexts. Luna handles nonengineering documents/images only; memory uses Terra medium. Dispatcher may run independent modules concurrently within their owned paths. Failed tests return to dispatcher, not core. Preserve the user's specified stopping conditions.

## Retained request semantics

Declare the task mode (`review`, `answer` or `change`) with the bridge `goal` tool. Pass all user flags and coverage requirements unchanged to dispatcher. For `[creative]`, engineering change mode is selected only after the independent ruling. Declare change mode after a qualifying ruling; otherwise close review with no follow-up exploration. Research considers at most three candidates with `area: feature`: the existing capability, the missing link, the code location, the user-visible value, and a falsifiable acceptance check. Rank by evidence strength, then direct user value, then reuse of what exists; select exactly one smallest increment or none. In creative mode `[no-architect]` is ignored. Under this mandatory role workflow it never authorizes skipping Astra HIGH planning for engineering.

Dispatcher preserves `## Coverage`, the output path `<store>/astra/plan.json`, `stage: "second_review"` research review evidence and `stage: "rewrite"` summaries where applicable. A `review-failed`, or `unreviewed` unit never counts as accepted. Keep failure context in the subtree and resume the same dispatcher ownership; core never reads worker reports or supplies fixes.

Dispatcher research may use `schema_path: "${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json"` for the complete audit; preserve any explicitly requested schema.
