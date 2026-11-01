# Dispatch policy

For whoever writes `codex_run` briefs. Workers never see this file; they see only the brief.

A worker stays small when the brief leaves nothing to infer. In 18 runs of delete, bug-fix and rename tasks with three-field briefs, `gpt-5.6-terra` finished every task in 2–7 commands and 1–13 changed lines, with no writes outside SCOPE, no backups and no added tests, under both the stock and a trimmed system prompt. Where a brief left a choice open, runs diverged: "rename `calc` everywhere" also renamed the test method `test_calc` in 2 of 6 runs.

## The brief

```
GOAL    one observable outcome, one sentence
SCOPE   the exact paths the worker may create, change or delete
VERIFY  one command; exit code 0 means done
```

Add `CONTEXT` only when the unit depends on something outside SCOPE: paste the upstream report or retrieved snippet itself, not a pointer to it. This is Pstack's collapsed brief for a one-command unit ([orchestrate](https://github.com/cursor/plugins/blob/99b609a72323b70204eab407c622f436e858e112/pstack/skills/poteto-mode/playbooks/orchestrate.md)); the full template is for coordinators.

## Before dispatch

- Decide every open choice in GOAL or SCOPE. If two reasonable workers could deliver different results, the brief is not finished.
- A field you cannot fill precisely means the unit is not ready. Split it, or decide the missing choice and record it; do not dispatch it as is.
- A check that guards an invariant gets a negative control: VERIFY includes a case that breaks the invariant and expects the check to fail. Without one the check can be vacuous: in the first Clotho experiment a TLA+ invariant passed model checking because no action ever set the variable it forbade, and a no-raw-rows test passed because its request never contained rows.
- VERIFY runs a test file, never inline code. Put the check in a test file listed in SCOPE and make VERIFY the test runner, such as `python -m unittest test_todo`. In the first end-to-end wish, both units used `python -c "..."` checks that broke on Windows shell quoting; the workers spent 106k and 157k input tokens working around them, one wrote and deleted a file outside SCOPE, and no test survived the unit.

## Write by subtraction

- No prohibitions, warnings or reminders. Naming a behaviour puts it in the worker's context. SCOPE and the bridge (sandbox, lean mode) bound what the worker can do.
- No persona, tone or background the unit does not use.


## Codex Rust assembly

The current external ASTRA medium is the only core. `dispatch` starts a read-only Terra medium coordinator, which calls independent Astra MAX research and returns `needs_intent_review` with its digest. Core pass/reject is explicit. Approval runs a fresh Astra HIGH planner, frozen baseline checks, ready scoped Terra units concurrently, mandatory SOL HIGH assembly, a fresh independent Astra HIGH verifier invoking frozen IDs through Rust, and Terra medium memory. Luna is available only for non-engineering artifacts. All leaves are fresh, lean and carry no root conversation. The Rust role profiles under `agents/` are resources, not native agent registrations.

Terminal completion is not task/test success. The root awaits an authority boundary with `max_wait_ms=0`; each execution has a finite budget. The same request id/input replays the saved boundary. Unknown writes retain reservations and require history/repair reconciliation; no polling core, GOAL continuation or promise to wake an offline host. A dedicated command-only AppServer launches with process cwd=scratch, with only scratch writable. Commands use explicit product paths or a prepared scratch copy. Generic projects finish at `ready_to_commit`; no remote, commit or Rust package release is required by the pipeline.
