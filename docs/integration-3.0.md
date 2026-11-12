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

- engine/agents/dispatcher.md: 9862 bytes
- engine/DISPATCH.md: 2824 bytes
- engine/skills/wish/SKILL.md: 8072 bytes
- total: 20758 bytes (cap 20813)

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
| `sessions` | unwired | No real `SessionStream`; shown as unavailable. |
| `unit_actions` | wired | `OrchUnitActions`. |
| `optimizer` | unwired | `None` retains the `ClaudeOptimizer` fallback. |
| `github` | unwired | `None` retains the `RealGh` fallback. |

Other seams and integrations:

- **Workshop `DecisionRouter` — wired:** `BackendRouter` calls the Claude backend and invokes the Laya observer via `laya::router::observer(home)` for `DecisionKind::Routing`, with `wish_id: None` and subject `workshop:<module>`. Laya is a best-effort shadow observer and never changes the choice. Curation excludes records without a wish ID, so these records never enter training.
- **Routines `WishLauncher` — unwired caller:** `ServiceLauncher` implements the real launcher, but no production scheduler calls the routines runner.
- **Modes promotion `Launcher` — wired:** `ServiceLauncher` starts wishes through a lazily connected service client. Quick launches use `LazyQuick`, which constructs `QuickLauncher` for the session project.
- **Modes chat/plaza `Provider` and `Router` — unwired:** `Unavailable` refuses calls because no real implementation exists.
- **Laya `DecisionObserver` — wired:** the configured observer is best-effort shadow; it can be a no-op when disabled or killed, and its prediction does not control the decision.
- **`SessionStream` — unwired:** no real implementation; the desktop presents it as unavailable.
- **Research-program role budgets — wired:** `Role::CouncilMember`, call name `program`, is checked before the program and before each member call; spend is recorded after each reply.
- **Graph feedback from 2b events — unwired:** plan item C15 was dropped by ruling 1. Existing explicit-feedback ingestion is separate.

Demo and tests keep fakes; an empty demo uses the default empty UX state. Production leaves optimizer and GitHub seams empty so their `ClaudeOptimizer` and `RealGh` fallbacks remain active.

## Suites

Run on Windows with `CARGO_HOME=E:/codex/.toolchains/cargo`, `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`. The suites overlapped while the first suite was running. Counts are from the initial runs, excluding nested child-test summaries; ignored tests are not counted as passes.

- `cargo test --locked --offline --no-fail-fast`: 1530 passed, 134 failed, 4 ignored. Failures:
  - `base::proc::tests::kills_spawned_process_group`
  - `base::proc::tests::no_bare_command_new_outside_the_helper`
  - `base::proc::tests::wait_times_out_for_running_process`
  - `session::launch::tests::boundary_reserves_for_child_and_stop_kills_it`
  - `session::launch::tests::project_notification_and_registered_owner`
  - `process_tree`
  - `tests/disk_guard.rs: disk_pause_stops_managed_batch_writes`
  - `tests/disk_guard.rs: headless_batch_wait_pauses_on_low_disk`
  - `tests/engine.rs: text_cases::required_phrases_are_present`
  - `tests/host.rs: drop_stops_a_running_wish`
  - `tests/host.rs: stop_kills_the_process_tree_and_blocks_the_goal`
  - `tests/liveness.rs: cancels_running_setup_only_for_driven_store`
  - `tests/local_channel.rs: clone_reads_while_writing`
  - `tests/local_channel.rs: many_sequential_clients`
  - `tests/local_channel.rs: rebind_after_drop_succeeds`
  - `tests/local_channel.rs: roundtrip_json_both_directions`
  - `tests/local_channel.rs: shutdown_unblocks_reader`
  - `tests/local_channel.rs: oversize_line_is_too_large`
  - `tests/local_channel.rs: eof_returns_none`
  - `tests/modes_quick.rs: launcher_accepts_quick_and_rejects_wish`
  - `tests/modes_quick.rs: end_to_end_and_receipt`
  - `tests/modes_quick.rs: failure_rejection_and_review_gate`
  - `tests/orch.rs: evidence_cases::external_failure_blocks_done_until_a_later_pass`
  - `tests/orch.rs: evidence_cases::records_and_gates`
  - `tests/orch.rs: lifecycle_cases::extra_commands_preserve_output_and_failures`
  - `tests/orch.rs: evidence_cases::identities_are_current_git_trees`
  - `tests/orch.rs: lifecycle_cases::ci_failure_opens_sol_like_a_local_failure`
  - `tests/orch.rs: lifecycle_cases::sol_and_extra_commands`
  - `tests/orch.rs: lifecycle_cases::a_failed_accept_blocks_a_unit_without_consuming_its_review`
  - `tests/orch.rs: lifecycle_cases::a_rebuilt_unit_can_be_superseded_once_its_files_really_changed`
  - `tests/orch.rs: lifecycle_cases::a_new_accept_keeps_the_review_of_unchanged_files`
  - `tests/orch.rs: lifecycle_cases::a_failed_review_is_not_erased_by_another_accept`
  - `tests/orch.rs: lifecycle_cases::review_is_bound_to_unit_and_reviewer`
  - `tests/orch.rs: lifecycle_cases::supersede_rejects_nonsense`
  - `tests/orch.rs: lifecycle_cases::temp_dir_is_not_scope_and_is_removed`
  - `tests/orch.rs: lifecycle_cases::dependencies_require_verified_unit`
  - `tests/orch.rs: lifecycle_cases::verify_command_refreshes_a_stale_unit`
  - `tests/orch.rs: lifecycle_cases::lifecycle_reuse_scope_and_limit`
  - `tests/orch.rs: lifecycle_cases::superseded_is_terminal`
  - `tests/orch.rs: lifecycle_cases::concurrent_workers_and_overlapping_scopes`
  - `tests/orch.rs: lifecycle_cases::verify_tail_keeps_last_2000_characters`
  - `tests/orch.rs: lifecycle_cases::tracks_dependencies_and_auto_accept`
  - `tests/orch.rs: lifecycle_cases::a_superseded_unit_keeps_its_history_and_stands_on_its_replacement`
  - `tests/orch.rs: lifecycle_cases::replacement_chains_resolve_to_their_last_unit`
  - `tests/orch_gates.rs: lifecycle_cases::extra_commands_preserve_output_and_failures`
  - `tests/orch_gates.rs: lifecycle_cases::sol_and_extra_commands`
  - `tests/orch_gates.rs: lifecycle_cases::ci_failure_opens_sol_like_a_local_failure`
  - `tests/orch_gates.rs: lifecycle_cases::a_failed_accept_blocks_a_unit_without_consuming_its_review`
  - `tests/orch_gates.rs: lifecycle_cases::a_new_accept_keeps_the_review_of_unchanged_files`
  - `tests/orch_gates.rs: lifecycle_cases::a_rebuilt_unit_can_be_superseded_once_its_files_really_changed`
  - `tests/orch_gates.rs: lifecycle_cases::superseded_is_terminal`
  - `tests/orch_gates.rs: lifecycle_cases::lifecycle_reuse_scope_and_limit`
  - `tests/orch_gates.rs: lifecycle_cases::review_is_bound_to_unit_and_reviewer`
  - `tests/orch_gates.rs: lifecycle_cases::a_failed_review_is_not_erased_by_another_accept`
  - `tests/orch_gates.rs: lifecycle_cases::dependencies_require_verified_unit`
  - `tests/orch_gates.rs: lifecycle_cases::verify_command_refreshes_a_stale_unit`
  - `tests/orch_gates.rs: lifecycle_cases::verify_tail_keeps_last_2000_characters`
  - `tests/orch_gates.rs: lifecycle_cases::supersede_rejects_nonsense`
  - `tests/orch_gates.rs: lifecycle_cases::temp_dir_is_not_scope_and_is_removed`
  - `tests/orch_gates.rs: lifecycle_cases::concurrent_workers_and_overlapping_scopes`
  - `tests/orch_gates.rs: lifecycle_cases::a_superseded_unit_keeps_its_history_and_stands_on_its_replacement`
  - `tests/orch_gates.rs: lifecycle_cases::tracks_dependencies_and_auto_accept`
  - `tests/orch_gates.rs: lifecycle_cases::replacement_chains_resolve_to_their_last_unit`
  - `tests/phase2a_acceptance.rs: fake_wish_survives_detach_and_reattaches_over_service`
  - `tests/proc_tree.rs: terminate_live_tree`
  - `tests/race_store.rs: a_stale_state_from_a_finished_unit_cannot_hide_a_newer_failing_review`
  - `tests/report_pack.rs: a_large_deletion_keeps_its_last_version_readable`
  - `tests/report_pack.rs: a_unit_that_deletes_a_file_named_in_scope_gets_a_snapshot`
  - `tests/report_pack.rs: fix_pack_lists_old_unwaived_serious_findings`
  - `tests/report_pack.rs: digest_names_sol_and_marks_built_units_unreviewed`
  - `tests/report_pack.rs: a_diff_over_the_budget_is_named_for_direct_reading`
  - `tests/report_pack.rs: review_stays_cross_family_with_a_pack`
  - `tests/report_pack.rs: a_change_git_cannot_show_is_still_named`
  - `tests/report_pack.rs: an_old_verdict_cannot_pass_a_new_pack`
  - `tests/report_pack.rs: a_unit_without_a_base_snapshot_is_reviewed_from_its_files`
  - `tests/report_pack.rs: digest_carries_what_the_final_report_lists`
  - `tests/report_pack.rs: digest_lists_changes_the_worker_did_not_report`
  - `tests/report_pack.rs: re_review_pack_contains_only_new_diff_and_unwaived_serious_findings`
  - `tests/report_pack.rs: pack_has_contract_changed_lines_and_new_files_only`
  - `tests/report_pack.rs: a_verdict_needs_a_pack_of_the_verified_tree`
  - `tests/report_pack.rs: module_digest_covers_only_that_module`
  - `tests/report_pack.rs: pack_needs_a_passing_verify_and_a_real_tree`
  - `tests/report_pack.rs: run_row_keeps_the_scope_as_it_was_before_the_worker`
  - `tests/report_pack.rs: digest_stays_bounded_however_large_the_run`
  - `tests/review.rs: another_codex_failure_is_an_error_and_never_reaches_agy`
  - `tests/review.rs: codex_auth_failure_falls_back_to_agy`
  - `tests/review.rs: codex_reviews_and_agy_is_not_used`
  - `tests/review.rs: codex_out_of_quota_falls_back_to_agy`
  - `tests/review.rs: the_prompt_is_bounded_and_holds_only_repository_text`
  - `tests/review.rs: neither_reviewer_available_is_skipped`
  - `tests/review_carry.rs: carries_through_accept_and_verify`
  - `tests/review_carry.rs: carry_requires_all_conditions`
  - `tests/review_carry.rs: partially_covered_diff_does_not_carry`
  - `tests/review_carry.rs: repeated_carry_merges_ids_and_accepts_superseded_units`
  - `tests/review_carry.rs: waivers_and_carried_reviews_obey_row_order_and_latest_check`
  - `tests/reviewer_modes.rs: failed_review_is_not_hidden_by_an_earlier_pass`
  - `tests/reviewer_modes.rs: pm_qa_hook_uses_its_own_verdict_and_binding`
  - `tests/reviewer_modes.rs: requirements_and_events`
  - `tests/reviewer_modes.rs: new_worker_run_requires_new_review_even_on_the_same_tree`
  - `tests/reviewer_modes.rs: verify_never_emits_verified_before_requirements_are_recorded`
  - `tests/reviewer_modes.rs: failed_latest_verify_never_starts_astra`
  - `tests/reviewer_modes.rs: requirements_are_bound_to_current_tree`
  - `tests/reviewer_modes.rs: same_family_default_and_failure_gate`
  - `tests/reviewer_modes.rs: matrix_and_engine_prompt`
  - `tests/reviewer_modes.rs: astra_finding_is_checked_once_and_both_verdicts_can_block`
  - `tests/scratch_env.rs: engine_verify_leaves_no_axiom_tmp`
  - `tests/service.rs: service_attach_detach`
  - `tests/service.rs: login_start_is_opt_in_and_resumes_interrupted_only`
  - `tests/service.rs: service_crash_releases_lock_and_preserves_resume`
  - `tests/service.rs: service_quit_cancel_keeps_work_running`
  - `tests/service.rs: service_reattach_returns_live_state`
  - `tests/service.rs: service_waits_for_space_then_resumes`
  - `tests/service.rs: tray_status_and_actions_follow_service`
  - `tests/session.rs: run_cases::interrupt_kills_process_tree`
  - `tests/setup_steps.rs: cancel_running_state_with_dead_pid`
  - `tests/setup_steps.rs: setup_refuses_store_inside_repo_and_reports_launch_failure`
  - `tests/setup_steps.rs: cancel_in_another_process`
  - `tests/setup_steps.rs: timeout_kills_tree_without_marker`
  - `tests/setup_steps.rs: index_collects_tracked_files_and_lost_supervisor_is_reported`
  - `tests/setup_steps.rs: start_skips_and_never_restarts_and_succeeds`
  - `tests/setup_steps.rs: cancel_from_thread_preserves_other_step_repo_and_cache`
  - `tests/setup_steps.rs: keep_then_cancel_all`
  - `tests/snapshot_ignored.rs: named_ignored_file_is_reviewed_and_invalidates_review_when_changed`
  - `tests/snapshot_ignored.rs: newly_created_named_ignored_file_is_in_tree_and_pack`
  - `tests/subdir_project.rs: project_relative_scope_pack_and_digest`
  - `tests/verdict.rs: default_verdict_and_findings_are_reported_and_truncated`
  - `tests/verdict.rs: default_missing_and_unknown_unit_are_reported`
  - `tests/verdict.rs: verify_returns_pack_with_verify_tree_in_text_and_json`
  - `tests/verdict.rs: explicit_verdict_is_saved_and_minor_only_output_is_unchanged`
  - `tests/verdict.rs: needs_only_exempts_untested_findings`
  - `tests/verdict.rs: partial_and_full_waivers_update_state`
  - `tests/verdict.rs: verify_pack_error_does_not_change_exit_and_failed_verify_has_no_pack`
  - `tests/verdict.rs: waive_requires_a_review_and_verdict_and_refuses_bad_inputs`
  - `tests/wish_flow.rs: a_long_batch_is_piloted_then_waited_on_without_a_model`
- `cargo test --locked --offline --no-fail-fast --features app --lib desktop::`: 867 passed, 3 failed, 0 ignored. Failures:
  - `src/lib.rs: desktop::live::accept::service_snapshot_rebuild_gap_resync_and_disk_pause`
  - `src/lib.rs: desktop::live::tests::service_background_reattaches_hosted_session_keys`
  - `src/lib.rs: desktop::ux::real::background_accept::detach_keeps_wishes_running_and_only_quit_stops_the_service`
- `cargo test --locked --offline --no-fail-fast --manifest-path tests/desktop-smoke/Cargo.toml`: 125 passed, 6 failed, 1 ignored. Failures:
  - `tests/service_attach.rs: window_close_keeps_fake_wish_running_and_reattaches`
  - `tests/service_attach.rs: quit_dialog_keep_working_leaves_service_running`
  - `tests/service_attach.rs: login_start_row_reflects_and_toggles`
  - `tests/service_attach.rs: quit_dialog_stop_and_quit_stops_wish_and_service`
  - `tests/service_attach.rs: disk_pause_reaches_report`
  - `tests/service_attach.rs: quit_dialog_stays_open_when_service_is_gone`

The individually retried named-pipe `os error 5` failures remained failures: the seven `local_channel` failures, `phase2a_acceptance::fake_wish_survives_detach_and_reattaches_over_service`, six `service` tests (`service_attach_detach`, `login_start_is_opt_in_and_resumes_interrupted_only`, `service_quit_cancel_keeps_work_running`, `service_reattach_returns_live_state`, `service_waits_for_space_then_resumes`, `tray_status_and_actions_follow_service`), all three desktop-lib failures, and smoke `window_close_keeps_fake_wish_running_and_reattaches`, `quit_dialog_keep_working_leaves_service_running`, and `disk_pause_reaches_report`. Other `os error 5` messages concerned scratch cleanup or Codex slots, not identified as named-pipe failures.

## Limitations

- Owner-only checks still open: real tray icon, real login registration.
- The paid live A/B was not run by this integration.
- SessionStream is unwired.
- Modes chat/plaza Provider and Router are unavailable (no real implementation).
- C15 graph feedback from 2b events: follow-up.
- connect_service still installs the private ServiceBackground in desktop::live.
- Short CJK graph queries s02 and s04 get no seeds.
- Windows named-pipe os error 5 flakiness under parallel service tests.
- os error 1455 (page-file exhaustion) was seen once during a run.

