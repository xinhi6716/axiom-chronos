# Integration 3.0

## Merges

### r9/p2b — f407deaa

readiness: ready

Nine conflict files retained both sides: `engine/agents/dispatcher.md` and `engine/skills/wish/SKILL.md` keep all behaviours of both sides, with wording compressed only. `src/bridge/codex_tool.rs`, `server.rs`, and `tools.rs` combine 2b role budgets, adjudicator, receipts, finding verification and integration graph sync, CONSTRAINTS prepend, explore, and typed memory. `src/cli/mod.rs` combines 2b commands with modes, workshop, extreme, and research-budget CLI. `tests/fakes/fake_codex.py`, `tests/module_map.rs`, and `tests/prompt_size.rs` use the integration long-TEMP base plus the 2b fake-script/fake-log; FAKE_GATE, interrupt gate, WAIT_FILE, and extreme tier are kept. ACCEPT 220 tests.

### r9/p2a — 21da09c7

readiness: ready

Conflicts: `Cargo.toml` keeps syn/proc-macro2 plus flate2 =1.1.11; `src/lib.rs` keeps the union of modules; `src/cli/mod.rs` adds launch/storage/serve arms. `src/session/run.rs` merges 2a service ownership, disk guard, preflight, and heartbeat with the 2b role budget Role::Core "session". `tests/prompt_size.rs` uses the integration base with the cap unchanged; lockfiles use the integration base with no version changed. ACCEPT 154 tests.

### r9/p4 — 069129d4

readiness: ready

P4 tip: `2587a38c`. `src/desktop/demo.rs` and `shell.rs` combine p4 UX hosts and earlier 2a/2c/5b wiring; p4 translation entries were re-added. In `src/desktop/doctor.rs`, `#[cfg(test)] mod ux_accept;` moved to the file end so 2a's source-guard test does not truncate code. Core ruling: `src/goal/mod.rs` received a one-line boundary edit in merge-p4: 2a's disk_pause test `goal::Interjection` literal gains `id: None, amendment_index: None`. ACCEPT 925 tests, 0 failed.

### Waivers and rulings

- merge-p4-5 finding 1 waived: shell close/quit coverage went to service-storage-seam.
- ss-seam-1 finding 1 waived: the paths are covered by `window_close_keeps_fake_wish_running_and_reattaches` and `service_attach`.
- First pass of the seam modules: five dispatchers wrote code outside the unit flow; each was redone through a reviewed unit after commit `327a5613`.
- Seam wiring is committed as `991ae137`.

## Prompt sizes

- engine/agents/dispatcher.md: 9568 bytes
- engine/DISPATCH.md: 3402 bytes
- engine/skills/wish/SKILL.md: 7786 bytes
- total: 20756 bytes (cap 20813)

Before this round: total 20768 bytes (cap 20813). After this round: dispatcher.md 9568, DISPATCH.md 3402, SKILL.md 7786; total 20756 bytes (cap 20813). Measured after normalizing CRLF to LF, as the test does.

## Offline graph evaluation

Offline re-evaluation uses the same scenarios as the pre-change baseline, not an independent acceptance set. Bytes are pretty explore JSON bytes, not tokens; counts are required evidence, conditions, and major findings present.

| Scenario | Seeds before → after | Bytes before → after | Evidence before → after | Conditions before → after | Findings before → after |
| --- | ---: | ---: | ---: | ---: | ---: |
| s01 | 0 → 2 | 469 → 1520 | 0 → 0 | 1 → 1 | 0 → 1 |
| s02 | 0 → 0 | 465 → 465 | 0 → 0 | 1 → 1 | 0 → 0 |
| s03 | 0 → 3 | 465 → 2099 | 0 → 2 | 1 → 1 | 0 → 1 |
| s04 | 0 → 0 | 465 → 465 | 0 → 0 | 1 → 1 | 0 → 0 |
| s05 | 0 → 1 | 469 → 909 | 0 → 0 | 1 → 1 | 0 → 1 |
| s06 | 0 → 2 | 464 → 2334 | 0 → 2 | 1 → 1 | 0 → 1 |
| s07 | 0 → 2 | 533 → 2080 | 0 → 2 | 1 → 1 | 0 → 1 |
| s08 | 1 → 3 | 856 → 2290 | 0 → 2 | 1 → 1 | 1 → 1 |
| s09 | 0 → 3 | 463 → 2357 | 0 → 2 | 1 → 1 | 0 → 1 |
| s10 | 0 → 2 | 543 → 2278 | 0 → 2 | 1 → 1 | 0 → 1 |
| s11 | 0 → 2 | 534 → 2308 | 0 → 2 | 1 → 1 | 0 → 1 |
| s12 | 0 → 2 | 470 → 2340 | 0 → 2 | 1 → 1 | 0 → 1 |

The paid live A/B was not run by this integration; frozen eval manifests, expected values, and byte budget were not edited. Short CJK queries s02 and s04 still get no seeds.

## Engine fixes

- Graph query seeding: word-level ranked seeds with short source snippets in `src/graph/query.rs`, within the byte budget; `tests/graph_seed_words.rs`, baseline in `tests/fixtures/graph_seed/baseline.json`.
- Cross-track supersede: orch allows superseding across tracks when the replacement is verified, emitting event `unit_superseded_cross_track` {old, new, old_track, new_track, basis}; an unverified replacement keeps the refusal "belongs to <new>, not <old>". `tests/orch_cross_track.rs`.
- Program role budgets: research-program members use Role::CouncilMember, call name "program", checked before the program and before each member call, spend recorded after each reply. `tests/program_role_budgets.rs`.
- Archive readers: usage::ledger, usage::period and evolve read archived transcripts. `tests/usage_archive.rs`.

## Seams

Production desktop wiring is installed by `ux::real::wiring::production` on a non-demo start. Construction does not connect to the service; service calls connect on demand.

| UX seam | Status | Adapter or reason |
| --- | --- | --- |
| `dual_core` | wired | `EventDualCore`, projects orch events and policy. |
| `turn_policy` | wired | `CodexTurnPolicy`, uses the Codex slots settings. |
| `interjections` | wired | `EngineInterjections` with `ServiceTransport`; sending requires the service. |
| `approvals` | wired | `EngineApprovals`. |
| `storage` | wired | `LedgerStorage`. |
| `background` | wired | `real::background::ServiceBackground`. After startup, `desktop::live::connect_service` installs its own private `ServiceBackground`, outside this wiring's scope. |
| `modes` | wired | `ModesEngine`; see launch and provider/router details below. |
| `sessions` | wired | `ServiceSessions` over the service snapshot and subscribe (`Client::snapshot`, `Client::subscribe`), client side only. Take-over, input and hand-back return a `Controller::Refused` reason because the protocol has no such operations. |
| `unit_actions` | wired | `OrchUnitActions`. |
| `optimizer` | unwired | `None` retains the `ClaudeOptimizer` fallback. |
| `github` | unwired | `None` retains the `RealGh` fallback. |

Other seams and integrations:

- **Workshop `DecisionRouter` — wired:** `BackendRouter` calls the Claude backend and invokes the Laya observer via `laya::router::observer(home)` for `DecisionKind::Routing`, with `wish_id: None` and subject `workshop:<module>`. Laya is a best-effort shadow observer and never changes the choice. Curation excludes records without a wish ID, so these records never enter training.
- **Routines `WishLauncher` — wired:** the production scheduler in `src/service/scheduler.rs` runs inside the background service (`Server::start`; `Server::start_with_scheduler` injects clock, launcher and approval for tests). Approval is `EnabledRoutineApproval`: it approves `launch_wish` only for a routine enabled in the store when the decision is made, and denies everything else, because no non-interactive person approval exists.
- **Modes promotion `Launcher` — wired:** `ServiceLauncher` starts wishes through a lazily connected service client. Quick launches use `LazyQuick`, which constructs `QuickLauncher` for the session project.
- **Modes chat/plaza `Provider` and `Router` — wired:** `ModelProvider` and `ModelRouter` in `src/modes/runners.rs` run over the real model runners. Failures are reported as `model_missing`, `quota_exhausted`, `budget_exceeded` or `malformed_reply`, never `unavailable` when a member's command resolves.
- **Laya `DecisionObserver` — wired:** the configured observer is best-effort shadow; it can be a no-op when disabled or killed, and its prediction does not control the decision.
- **`SessionStream` — wired:** `ServiceSessions` (see the table row).
- **Research-program role budgets — wired:** `Role::CouncilMember`, call name `program`, is checked before the program and before each member call; spend is recorded after each reply.
- **Graph feedback from 2b events — wired:** plan item C15. Events are ingested with the correlation id recorded in `FeedbackEvent.source` (`<events.jsonl locator>#<correlation id>`); the struct is unchanged and dedup by source holds.
- **Workbench paper slot — wired:** `PaperHandoff` is a snapshot consumer that feeds phase 8 evidence.
- **Hydrogen/Flash and Argon pools — wired:** pool membership by model-id prefix; an exhausted pool is recorded with `until_ms` (provider reset time, else now plus 60 minutes). Argon takes part in every council round (setting `argon_every_round`, default on).
- **Vault and memory slot — unwired, not in this version:** phase 6 is shelved; the slot shows "not in this version".
- **Optimizer (`ClaudeOptimizer`) and github (`RealGh`) — unwired:** left on the fallbacks from the earlier round; no real adapter was in scope this round.

Demo and tests keep fakes; an empty demo uses the default empty UX state. Production leaves optimizer and GitHub seams empty so their `ClaudeOptimizer` and `RealGh` fallbacks remain active.

## Suites

Suites text (final, sequential runs, Windows, CARGO_HOME=E:/codex/.toolchains/cargo, CARGO_BUILD_JOBS=2, CARGO_INCREMENTAL=0, the three suites run one after another and never overlapping):
- `cargo test --locked --offline --no-fail-fast`: 2429 passed, 0 failed, 6 ignored (sum of per-binary results, nested child summaries excluded).
- `cargo test --locked --offline --no-fail-fast --features app --lib desktop::`: 960 passed, 0 failed, 0 ignored.
- `cargo test --locked --offline --no-fail-fast --manifest-path tests/desktop-smoke/Cargo.toml -j 1`: 184 passed, 0 failed, 1 ignored.
- Linux (WSL), `cargo test --locked --offline --no-fail-fast`: 2406 passed, 14 failed, 12 ignored; all failures are host limits, not product regressions (see below).

Regressions fixed (each reproduced deterministically, then fixed under its module with VERIFY and review):
1. engine text_cases::required_phrases_are_present and wish_flow a_long_batch_is_piloted_then_waited_on_without_a_model: engine prompt wording lost in the p2b merge, restored in engine/agents/dispatcher.md (unit fix-p2b-prompts-2, which supersedes fix-p2b-prompts).
2. base::proc::tests::no_bare_command_new_outside_the_helper: a bare Command::new in src/desktop/ux/real/modes_accept.rs, routed through the helper (unit fix-modes-command).
3. r10_integration prompt_total_matches_files: the recorded dispatcher.md size was 9862, the file is 9872; the record now reads 9872 and total 20768 (unit gates-prompt-sizes).
4. Four prompt/doc phrase tests: required wording restored (unit fin-prompts-1).
5. CRLF renormalisation of tests/fixtures/paper: LF bytes restored, protected by `.gitattributes` `-text` and tests/paper_fixture_bytes.rs (unit fin-paper-eol-2).
6. Host test Python lookup corrected (unit fin-python-1).
7. Desktop shell source-guard tests, startup seams and live reload corrected (unit fin-desktop-1).
8. Desktop pool publishing wired through `live::quota::publish_pools`, called from src/desktop/live.rs (unit fin-pools-1).
9. `base::proc::pid_alive` treated PIDs above i32::MAX as alive on Linux; corrected (unit fin-pidalive-1).
10. Doctor-page walk tests clicked a Preferences switch below a 900 px window after pool rows grew the Budgets card; the test window is now taller (unit fin-walk-1).

Environmental and load-sensitive, not regressions:
- Under overlapped suites the first pass showed 134 main, 3 desktop and 6 desktop-smoke failures, mostly named-pipe os error 5 and os error 1455 and process-tree timing; none remain when the suites run sequentially.
- tests/setup_steps.rs cancel_from_thread_preserves_other_step_repo_and_cache: the "other" step sleeps 3 seconds and had finished when cancel returned on a loaded machine (1 failure in 5 runs; 4 passes alone).
- tests/disk_guard.rs low_disk_cleans_then_pauses_with_distinct_reason: failed once in a full run (the binary took 20 s instead of 2 s); passed alone twice.
- desktop-smoke ux_interjection: one STATUS_ACCESS_VIOLATION in one full run; passed alone 3 of 3 and in the other full run.
- tests/scheduler_wall_clock.rs failed in one full run and passed in the next.
- Orch lifecycle_cases ci_failure_opens_sol_like_a_local_failure ("database is locked"), lifecycle_cases sol_and_extra_commands, and program_role_budgets mid_run_exhaustion_stops_later_requests failed once in an earlier full run (2425 passed, 3 failed); all passed alone and in the clean full re-run (2429 passed, 0 failed).
- WSL host limits: ONNX Runtime is not installed (laya_infer, laya_ort and laya_router); laya_ort laya_runtime_directory_is_git_ignored, paper_fixture_bytes and scratch_outside_repo call `git` in a worktree whose `.git` file points to a Windows path; storage_ledger measure_walk_does_not_hold_write_lock has a Linux timing failure because the 12000-file walk finishes before register returns.

## Limitations

- Owner-only checks still open: real tray icon, real login registration.
- The paid live A/B was not run by this integration.
- Vault and memory slot: not in this version (phase 6 shelved).
- Optimizer and github seams stay on their fallbacks.
- Astra read-only tools are enforced only by the prompt (`src/codex/run.rs`).
- The Claude runner charges no usage on unparseable replies (`council/member.rs`).
- connect_service still installs the private ServiceBackground in desktop::live.
- Short CJK graph queries s02 and s04 get no seeds.
- Windows named-pipe os error 5 flakiness under parallel service tests.
- os error 1455 (page-file exhaustion) was seen once during a run.

