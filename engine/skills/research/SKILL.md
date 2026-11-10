---
name: research
description: Turn a research question into a ruled research plan. Astra drafts it at maximum effort, `chronos:critic` reviews it for overreach, and you make the final ruling. Started by `axiom research`.
disable-model-invocation: true
argument-hint: <question>
---

# Research

Question: $ARGUMENTS

1. If the question is empty, ask for it and stop.
2. Name the program with a lowercase slug of the project directory's name and call the bridge `orch` tool with `["init"]` for it. Research files go in `<store>/research/`.
3. Read `${CLAUDE_PLUGIN_ROOT}/research/plan.schema.json` and `${CLAUDE_PLUGIN_ROOT}/research/review.schema.json`.
4. Give the `chronos:astra` agent the program slug, the project directory, the plan schema path, the output path `<store>/research/plan.json`, web search on, and this brief:

   ```
   GOAL      A research plan for the question below, with every field of the output schema filled.
   For empirical questions, put the claim and alternative explanations in `claims`; put the baseline, separation of development and acceptance data, and a threshold fixed before running in `design`; put the counterexample that would refute the claim in `falsification`. Literature questions are exempt from experiments.
   QUESTION  <the question, verbatim>
   ```

   If it reports a failure or a model that is not an Astra model, report that and stop.
4a. Council: only when the system prompt says advanced research is on and the bridge `council` tool exists; otherwise skip this step. Call `council` with `project` the slug, `cwd` the project directory, `plan_path` `<store>/research/plan.json`, `schema_path` `${CLAUDE_PLUGIN_ROOT}/research/plan.schema.json` and `brief` the exact brief Astra was given. Read only its reply, a bounded digest; never open the members' position files. State its `note` under the final plan. Unless `outcome` is `ordinary`, add `digest_path` to the Critic's prompt in step 5 with the sentence: Also read the council digest at that path; treat its open claims as claims about this plan. In step 6 rule on each `open` claim and record them with `council` `action: "rule"`, the same `project` and `plan_path`, and `rulings` of `id`, `decision` (`upheld`, `rejected` or `narrowed`) and `reason`.
5. Give the `chronos:critic` agent the review schema, the plan path and a prompt made of this paragraph, then the question:

   Review this research plan against its question. Report each item that goes beyond the question (out_of_scope), claims more than its evidence supports (overclaim), cannot be shown wrong by any test the plan lists (unfalsifiable), or adds machinery the question does not need (complexity). Check for data leakage between development and acceptance data and thresholds changed after results. Then give a verdict.

   Write the review to `<store>/research/review.json`.
6. Rule on each finding yourself: uphold it or overrule it, with a one-line reason grounded in the question and the plan. Do not send anything back to Astra or the critic. Apply the upheld findings to the plan by removing or narrowing what they name, and write the result to `<store>/research/plan-final.md`, and your rulings to `<store>/research/ruling.md`.
7. Show the final plan in the question's language. Under it, list the critic's verdict, each finding with your ruling and reason, and the Astra model and its input tokens.
