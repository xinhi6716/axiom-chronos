# Dispatch policy

Workers see only the `codex_run` brief, not this file.

Leave nothing to infer: open choices let workers diverge.

## The brief

```
GOAL    one observable outcome, one sentence
SCOPE   the exact paths the worker may create, change or delete
VERIFY  one command; exit code 0 means done
```

SCOPE lists paths only; notes such as "(new)" go in CONTEXT. SCOPE may span lines or bullets; every path counts. Write all module briefs in one call: a single Bash heredoc, or Write for one unit.

VERIFY runs under `cmd` on Windows and `sh` on Linux: write commands without `set` or `%VAR%`, and rely on the inherited CARGO_HOME.

Workers and VERIFY get private scratch (TMP, TEMP, TMPDIR, pytest `--basetemp`) automatically; workers' scratch is a sandbox writable root.

Dispatch a registered unit with `project`, `unit`, `role` (`terra` or `sol`), `cwd`, and `cycle: true`, or register and dispatch a new one in the same call by adding `track` and `brief_file`; never add `brief` or `save_to`. Brief mode is for Astra only.

Between GOAL and CONTEXT, `TIER terra|sol|astra` selects the tier; `EFFORT low|medium|high|xhigh|max` selects effort. The bridge reads them from the registered brief, so `codex_run` takes no argument for them, and the worker never sees them. Legacy syntax: `EFFORT` (`low`, `medium` or `high`) sets the reasoning effort, and `TIER luna` runs a terra unit on the luna model. Luna remains legacy for terra units only, never in maps.

Put these worker-visible sections between GOAL and CONTEXT:

```
CONTRACT
<observable promises>
TEST_MATRIX
<id> | <behaviour|edge|failure> | <file>::<case> | <assertion> | <command>
```

Test files must be in SCOPE. Both sections are mandatory in routed stores; a present matrix is always enforced. Write tests first: the engine records RED before implementation, and VERIFY runs the matrix. RED must report a failing test matching each row's case, not a compile error, missing command or bare nonzero exit. Do not change matrix test files during implementation (`red_tampered`).

Register a fix with `unit add --fixes <old>`: orch climbs one routing step and writes TIER/EFFORT into its brief. A plan exception requires `--routing <tier>/<effort> --reason <text>` together; saturation without it gives `escalation_saturated`.

In `CONTEXT`, include the graph packet. For dependencies outside SCOPE, paste the upstream unit result as its HANDOFF block or the retrieved snippet itself, not a pointer.

## Before dispatch

- Decide every open choice in GOAL or SCOPE. If two reasonable workers could deliver different results, the brief is not finished.
- A field you cannot fill precisely means the unit is not ready. Split it, or decide the missing choice and record it; do not dispatch it as is.
- A check guarding an invariant gets a negative control: VERIFY includes a case that breaks it and expects the check to fail.
- VERIFY runs a test file, never inline code. Put the check in a test file listed in SCOPE and make VERIFY the test runner, such as `python -m unittest test_todo`.

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
