# Caller authority repair — 0.2.2-codex.dev.7

This is a direct repair of the broken dispatcher workflow, not a completed product-optimization round (0/5).

Core relays intent → Terra dispatcher → Astra MAX research → core intent-only pass/reject → separate Astra HIGH planning → Terra implementation → Sol HIGH assembly → separate Astra HIGH testing → core submission.

Rust persists the phase and research revision under `authority/<session>/state.json`. Host supplies caller identity; model-supplied actor names cannot grant privileges. Core receives only goal, workflow and dispatcher calls. Attached raw bridge calls cannot bypass Host checks. Approval unsubscribes the ephemeral research thread and drops Host context metadata; artifacts remain for planning. App-server controls its idle memory reclamation; immediate physical unloading is not asserted. Answer/review mode returns to research after approval and does not unlock engineering.

Verified units use Host lifecycle `test_approve`, completion and release events; no technical core approval is fabricated. A final tester can unlock submission only after current unit, package and full-test evidence exists. Optional presentation rewriting is not engineering acceptance. Submission rechecks both test evidence and the final tester's source identity. Source changes require re-verification. Completed invalid plans return to planning; uncertain transport termination remains reserved, rather than assuming a dead worker.

Limits: these permissions govern Chronos-owned operations, not the desktop host's external terminal tools. `workflow submit` records tested acceptance; it is not itself a Git commit/push operation. No automatic five-round iteration or host GOAL is enabled by this repair.

Validation distinguishes deterministic model/RPC fixtures from real-model testing. Fixture coverage includes core bypass attempts, stale research revisions, research unsubscribe, restart at planning, real Cargo build/package/full tests, lifecycle unit release and final tester-to-core submission. Live acceptance must use a fresh desktop task without mid-test instructions; it is not established by fixture success alone.

Live dev.2 failed at research approval: `thread/archive` rejected an ephemeral thread with `no rollout found`. Dev.3 uses acknowledged `thread/unsubscribe` and prohibits reusing that context. Reference: https://learn.chatgpt.com/docs/app-server#unsubscribe-from-a-loaded-thread. The failed task is retained as negative evidence.

Dev.3 live testing reached implementation, Sol assembly and independent testing, but exposed ordinary repair returning to core and a missing failed-test retry authority. Dev.4 holds one caller command under an OS lock, continues the same dispatcher context at completed internal stages, stops boundedly on no progress, and routes normal verified test findings through internal test_reject to a new repair checkpoint. Rejection never grants release. Brief/map machine syntax and minimum-scope instructions were clarified from the observed failures. Host-imposed model re-entry during a pending tool is not claimed to be controlled by the Rust lock.

Dev.4 live testing kept core blocked without busy waiting but failed delivery: repeated identical successful ACCEPT discarded the prior source-bound independent review. Dev.5 preserves review across an unchanged successful acceptance chain; failed acceptance, changed command/source, or failed review still invalidates it. Regression tests cover these negative controls. The dispatcher no longer repeats the pipeline acceptance or confuses scoped and whole-source identities. Live acceptance remains pending.

Dev.5 live testing exposed a preflight ordering defect: invalid research save_to reserved research_running before any worker was launched, making a corrected request unauthorized. Dev.6 validates local arguments before reserving research/planning; a real Host callback fixture retries invalid research destination and invalid planner cwd successfully. Unknown post-launch termination remains fail-closed.

Dev.6 live testing completed CLI implementation and five tests but failed final acceptance: research promised project-local build artifacts despite Host isolated short-path builds, then dispatcher incorrectly requested an implementation attempt for unchanged-source repackaging. Dev.7 exposes the existing Host artifact contract before research/planning, clarifies typed verification repair, and persists failed final-tester reports as well as successful ones. Explicit user location requirements remain binding; previous failed acceptance is not retroactively passed.

## Live acceptance — dev.7

Fresh desktop task `01a19366-4fef-7992-a512-cf45d495d0f0` completed without observer follow-up instructions. Session `3779f641dabd9ab8eaaaed61b28fc2f6` persisted `submitted`; its Chronos goal persisted `done`. Total 773.500 s; dispatcher commands remained pending for 282.332 s (research) and 441.185 s (planning through final acceptance). The core issued zero status polls and zero engineering calls; its only shell read loaded the skill. There was one host-side waiting commentary during the pending command, so complete suppression of host model re-entry is not claimed. One pass verdict with a reason was correctly rejected and retried without a reason.

The dispatcher completed three internal attempts: implementation, test-case repair, then unchanged-source verification/repackaging. Terra HIGH, Sol HIGH and independent Astra HIGH receipts were recorded. Six CLI integration tests and default-verified Cargo packaging passed. Final source was `tree:44888c8aeaf28dd1b576c917be52a503a6d98e4a`; the observer independently matched the 3,251-byte package SHA256 `e532fca6bed6b5cc015718c10b51c59e2e4aa24e20109c9e18d0dd3792f94832`. These are Windows results, not Unix/release execution evidence. Internal attempt counts are not the requested five optimization rounds (still 0/5).

Dev.7 affected regressions: session 55, engine 9, release 1, final-tester positive 1 and negative 1, all passing. Prior unchanged dev.6 bridge suite: 77 passing, 1 ignored. Prior unchanged acceptance-state dev.5 orch suite: 95 passing. Rust release build and Cargo package verification passed. Detailed receipts and observer call audit are under `experiments/chronos-role-flow-dev7` in the containing project, with previous failed task evidence retained.

## Repeat acceptance and self-iteration readiness — 2026-11-01 (Asia/Taipei)

The unchanged dev.7 executable (SHA256 `842b97453f93e8cba202c1e5e90b04fe57f2e4c06ea7fedbf08975fda879f408`) passed the same plain-user CLI task in another clean project and isolated state home. Fresh desktop task `01a19374-cc77-7c33-8b94-0ac3912067fa`, session `d1caf52351f321737fcbde1def440783`, completed in 695.116 s. Dispatcher calls blocked for 281.918 s and 373.923 s; the core issued no status polls or engineering calls, and the observer sent no follow-up instructions. One host-side waiting commentary and one rejected pass-with-reason retry remained. Dispatcher handled an independent tester's missing-test findings and completed two internal attempts. Five Windows CLI black-box tests passed; full tests and default-verified Cargo packaging passed. State was `submitted` / `done`. Observer independently matched the final package hash `b4e3e26efe37600b3ddb49688051bcece4ed520e17fa5e64c182a0e15675f61b` against the receipt bound to `tree:67077adc8b65c21d51958fe43e72bd47760617c6`.

Pre-push `cargo test --all-targets`: 364 passed, zero failed, 3 ignored across 18 test binaries. `cargo package --allow-dirty --locked` verified successfully; generated local data-home is excluded from Git/package inputs. This is two successful clean-environment repetitions of one exposed task, not a general task-success estimate.

Readiness judgment: sufficient to begin bounded self-iteration with core intent/major-decision and per-round submission gates. Not evidence for unattended cross-version five-round self-iteration. The complete commit/push/install/new-bridge-version handoff, rollback after interrupted upgrade, and large multi-module workload have not been accepted live. Each future round must verify the running binary hash/version and new bridge connection before executing the next round. Existing product policy/checkpoint tests are not substitutes for this cross-version acceptance. Five requested optimization rounds remain 0/5; this repair/retest does not start them.


## dev.8: cross-platform CI repair

The dev.7 push failed on Ubuntu (25 bridge failures); the default matrix
fail-fast policy cancelled Windows. Earlier Windows local tests and two live
desktop tasks did not establish Linux acceptance.

## Changes

- Explicitly unlock the stable store `flock` before closing its descriptor.
  A concurrent Unix fork can inherit the same open file description; closing
  only the parent's descriptor lets the child retain the lock until exec/exit.
  The process-local mutex, stable inode and cross-process exclusion remain.
  A Unix regression forks while locked and keeps the child alive through a pipe
  while the parent reacquires the released lock, without timing assumptions.
- Await bounded, parallel interrupt acknowledgements before cancellation returns.
  Previously detached sends could lose the race against transport shutdown.
  Acknowledgement is not terminal confirmation; turn owners retain that check,
  and acknowledgement failure marks the server unconfirmed.
- Install ripgrep in both CI environments and run both matrix jobs even if one
  fails. Use the existing Python resolver in the authority integration test.
- Improve fixture diagnostics. The verifier-cancellation fixture stays pending
  after the killed verifier returns, so it actually tests model interruption;
  the separate `attached-late` scenario still tests late completion. No assertion
  accepts a cancelled request as successful or removes terminal checks.
- Bump Rust and plugin manifests together to `0.2.2-codex.dev.8`.

Scope is CI/runtime repair, not a completed product optimization round. Real
five-round cross-version self-iteration remains unaccepted.

## Local validation

Ubuntu 22.04 / Rust 1.99.1: `cargo test --locked` passes all 366 tests,
with 3 existing ignored tests. Includes the actual role authority sequence,
parallel bridge tests, interruption, recovery and the new fork regression.
GitHub's Ubuntu/Windows matrix must also pass before this release is installed.
