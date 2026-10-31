---
name: research
description: Route research through dispatcher and independent Astra MAX contexts, with core reviewing intent only.
disable-model-invocation: true
argument-hint: <question>
---

# Research

Question: $ARGUMENTS

The current caller is the sole root host; do not start another core model. Declare answer mode through `goal`. Core never writes, runs engineering tools, designs or makes technical rulings.

Delegate the original question to `chronos:dispatcher`. Ask dispatcher to preserve this plan-only procedure in its local handoff:
1. Independent Astra MAX drafts a research plan with `schema_path: "${CLAUDE_PLUGIN_ROOT}/research/plan.schema.json"`, saved to output path `<store>/research/plan.json`.
2. The second-review stage uses a fresh independent Astra MAX and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`, checking out_of_scope, overclaim, unfalsifiable and complexity. Save `<store>/research/review.json`.
3. A fresh Astra MAX writes `<store>/research/plan-final.json`, ruling on each finding with evidence, unchanged coverage, unknowns and next action. Technical rulings belong to this researcher; core must not do it.

Each researcher result returns a Rust `intent_review` boundary. Core checks only user-intent compatibility and calls `workflow` action `verdict` with current revision and `pass`, or `reject` with the shortest conflict reason. In answer/review mode, pass archives that research context and returns to research; it never unlocks engineering. Relay the verdict and requested next stage to dispatcher. Keep full artifacts in the store rather than core context.

After the requested final stage succeeds and passes intent review, return bounded status, artifact paths, actual model and usage, then close `goal` done. If explicitly requested, display the saved final plan without redesigning it. Preserve actual failure; never claim missing review completed. No engineering, full-suite or memory ritual for research-only work.
