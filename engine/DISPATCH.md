# Dispatch policy

For whoever writes `codex_run` briefs. Workers never see this file; they see only the brief.

A worker stays small when the brief leaves nothing to infer. In 18 runs of delete, bug-fix and rename tasks with three-field briefs, `gpt-5.6-terra` finished every task in 2–7 commands and 1–13 changed lines, with no writes outside SCOPE, no backups and no added tests, under both the stock and a trimmed system prompt. Where a brief left a choice open, runs diverged: "rename `calc` everywhere" also renamed the test method `test_calc` in 2 of 6 runs.

## The brief

```
GOAL    one observable outcome, one sentence
SCOPE   the exact paths the worker may create, change or delete
VERIFY  one command; exit code 0 means done
DELIVERY implementation
OUTPUTS exact relative output paths, separated by spaces
```

Machine syntax is literal: keep each field's value on its header line, begin the brief with GOAL, and use only paths in SCOPE/OUTPUTS (no explanations or semicolons). DELIVERY is exactly `implementation`, `verification` or `measurement`, never a sentence. OUTPUTS names actual required files within SCOPE. Put explanation in GOAL or indented CONTEXT. For example:

```
GOAL Implement the approved CLI contract and its integration tests
SCOPE src/main.rs tests/cli.rs README.md Cargo.lock
VERIFY cargo test --test cli --locked --offline
DELIVERY implementation
OUTPUTS src/main.rs tests/cli.rs README.md Cargo.lock
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


## Ownership and recovery

The external core holds intent/authorization, not design or worker reports. Independent Astra max owns planning and technical rulings; Terra high dispatcher owns the engineering subtree, Terra high modular workers, Sol high assembly, Terra medium memory, Luna only nonengineering content. Read-only dispatcher native tools do not prohibit its supplied Host dynamic dispatch tools in inherited change mode. No additional core. Preserve authorized uncommitted edits; include referenced nested tests/helpers in scope.

Keep contracts, failure evidence, identities and next actions in the owner store at stage boundaries. Send bounded status/summary/artifact pointers upward (at most 8 KiB; core summary at most 100 words); pass only necessary actual upstream excerpts downward. Unconfirmed turns retain quarantine until external termination confirmation, never blind retry. Model completion, unit evidence, assembly and installed-product checks are distinct.

Independent Astra HIGH uses `role: "astra-verify"`, explicit `sandbox: "read-only"` and `task_kind: "verification"` or `"measurement"`; assign only verification or measurements, never product modification or improvement. Astra MAX remains the research role. The report distinguishes requested effort from actual effort, which may be unknown.
