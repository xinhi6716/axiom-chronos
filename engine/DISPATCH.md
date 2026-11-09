# Dispatch policy

For whoever writes `codex_run` briefs. Workers never see this file; they see only the brief.

A worker stays small when the brief leaves nothing to infer. In 18 runs of delete, bug-fix and rename tasks with three-field briefs, `gpt-5.6-terra` finished every task in 2–7 commands and 1–13 changed lines, with no writes outside SCOPE, no backups and no added tests, under both the stock and a trimmed system prompt. Where a brief left a choice open, runs diverged: "rename `calc` everywhere" also renamed the test method `test_calc` in 2 of 6 runs.

## The brief

```
GOAL    one observable outcome, one sentence
SCOPE   the exact paths the worker may create, change or delete
VERIFY  one command; exit code 0 means done
```

SCOPE lists paths only; notes such as "(new)" go in CONTEXT.

VERIFY runs under `cmd` on Windows and `sh` on Linux: write commands without `set` or `%VAR%`, and rely on the inherited CARGO_HOME.

Dispatch a registered unit with exactly `project`, `unit`, `role` (`terra` or `sol`) and `cwd`; never add `brief` or `save_to`. Brief mode is for Astra only.

A registered brief can run its unit cheaper with two optional lines between GOAL and CONTEXT: `EFFORT` (`low`, `medium` or `high`) sets the reasoning effort, and `TIER luna` runs a terra unit on the luna model. Use them only when the wish or the plan asks for a cheaper run of that unit, such as reading papers into notes. The bridge reads them from the registered brief, so `codex_run` takes no argument for them, and the worker never sees them. Any other value is refused when the unit is registered. Without them the unit keeps its role's model and the default effort. `TIER luna` alone keeps luna's own default effort, which can be higher than terra's, so set both for the cheapest run.

Add `CONTEXT` only when the unit depends on something outside SCOPE: paste the upstream unit result as its HANDOFF block or the retrieved snippet itself, not a pointer to it. This is Pstack's collapsed brief for a one-command unit ([orchestrate](https://github.com/cursor/plugins/blob/99b609a72323b70204eab407c622f436e858e112/pstack/skills/poteto-mode/playbooks/orchestrate.md)); the full template is for coordinators.

## Before dispatch

- Decide every open choice in GOAL or SCOPE. If two reasonable workers could deliver different results, the brief is not finished.
- A field you cannot fill precisely means the unit is not ready. Split it, or decide the missing choice and record it; do not dispatch it as is.
- A check that guards an invariant gets a negative control: VERIFY includes a case that breaks the invariant and expects the check to fail. Without one the check can be vacuous: in the first Clotho experiment a TLA+ invariant passed model checking because no action ever set the variable it forbade, and a no-raw-rows test passed because its request never contained rows.
- VERIFY runs a test file, never inline code. Put the check in a test file listed in SCOPE and make VERIFY the test runner, such as `python -m unittest test_todo`. In the first end-to-end wish, both units used `python -c "..."` checks that broke on Windows shell quoting; the workers spent 106k and 157k input tokens working around them, one wrote and deleted a file outside SCOPE, and no test survived the unit.

## Write by subtraction

- No prohibitions, warnings or reminders. Naming a behaviour puts it in the worker's context. SCOPE and the bridge (sandbox, lean mode) bound what the worker can do.
- No persona, tone or background the unit does not use.


## HANDOFF

Use a short HANDOFF block with exactly these five fields:

```
HANDOFF
RESULT  outcome and interface
PATHS   changed paths
COMMANDS  command and result
OPEN    unresolved items
EVIDENCE  report/verify paths
```
