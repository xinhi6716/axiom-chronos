# Council

The council is opt-in and remains off by default. The engine conducts three phases:

1. **A — Plan.** Astra speaks first in every round; its initial turn is the existing plan. Fable sees that turn, agrees or rebuts per claim, and explicitly approves or refuses the whole plan. Astra revises or defends, then Fable responds again. Approval closes the phase only when no rebuttal remains open.
2. **B — One review.** Argon alone reviews exactly once after A, using neutral position labels. It sees the agreed plan or both final positions with open disputes, agrees or objects per claim, and may add claims. With no objection the discussion ends. Missing, failed or capped Argon means no review; there is no retry or phase C.
3. **C — Replies, only for objections.** Astra speaks first, Fable second. Both settle each objection by accepting a concrete change or rejecting it with a reason. Argon is never called again. Its claims and stances survive for the core even when the replies reject them.

Each dialogue phase stops at agreement, a spent budget, a failed turn or `AXIOM_COUNCIL_ROUNDS` (positive integer, default 3). A failed Fable turn ends that phase with its reason. The original Astra plan always survives. Unresolved disputes go to the core. `council.json` preserves phase, round and member for claims, stance history and turn outcomes; position files use `<phase>-r<round>-<member>.json`. The digest lists ordered turns, phase agreement and open disputes. Long identical sequences use `n` through `through` ranges: every round in that range has the listed ordered speakers and outcomes. Dropped claims and degraded members are reported. The core rules after these phases, as below.

Budgets: `AXIOM_FABLE_CAP_USD` (default $5, positive finite dollars, at most $200), `AXIOM_ASTRA_TOKEN_CAP` (default 200000), `AXIOM_ARGON_TOKEN_CAP` (default 400000, one review), and `AXIOM_COUNCIL_ROUNDS`. Token caps and rounds are positive integers. Bad values refuse startup with the variable and received value; above $200 is probably a typo. `--fable-cap`, `--astra-cap`, `--council-rounds` require `--advanced-research` and override the child environment. Desktop Settings persists all four values; the New wish switch still starts off and shows the effective budgets when on.


Before the Critic review:

Call `council` with `project` the slug, `cwd` the project directory, `plan_path` `<store>/astra/plan.json`, `schema_path` `${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json` and `brief` the exact brief Astra was given. Read only its reply, a bounded digest; never open the members' position files. Say its `note` plainly in the report. `outcome` `ordinary` means the council did not sit: go on as an ordinary wish. Otherwise include `digest_path` in the Critic's prompt below, with this sentence: Also read the council digest at that path; treat its open claims as claims about this plan.

After the Critic review:

If the council sat, rule on each of its `open` claims too and record them with `council` `action: "rule"`, the same `project` and `plan_path`, and `rulings` of `id`, `decision` (`upheld`, `rejected` or `narrowed`) and `reason`; while the reply's `more_open` is true, rule its `next` claims too. Nothing goes back to the members.
