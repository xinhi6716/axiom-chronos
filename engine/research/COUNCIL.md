# Council

Before the Critic review:

Call `council` with `project` the slug, `cwd` the project directory, `plan_path` `<store>/astra/plan.json`, `schema_path` `${CLAUDE_PLUGIN_ROOT}/research/audit.schema.json` and `brief` the exact brief Astra was given. Read only its reply, a bounded digest; never open the members' position files. Say its `note` plainly in the report. `outcome` `ordinary` means the council did not sit: go on as an ordinary wish. Otherwise include `digest_path` in the Critic's prompt below, with this sentence: Also read the council digest at that path; treat its open claims as claims about this plan.

After the Critic review:

If the council sat, rule on each of its `open` claims too and record them with `council` `action: "rule"`, the same `project` and `plan_path`, and `rulings` of `id`, `decision` (`upheld`, `rejected` or `narrowed`) and `reason`; while the reply's `more_open` is true, rule its `next` claims too. Nothing goes back to the members.
