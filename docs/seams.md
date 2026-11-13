# Seam inventory

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
- **Hydrogen/Flash and Argon pools — wired:** pool membership by model-id prefix; an exhausted pool is recorded with `until_ms` (provider reset time, else now plus 60 minutes). Argon takes part in every council round (setting `argon_every_round`, default on). The desktop publishes pool state through `live::quota::publish_pools`.
- **Vault and memory slot — unwired, not in this version:** phase 6 is shelved; the slot shows "not in this version".
- **Optimizer (`ClaudeOptimizer`) and github (`RealGh`) — unwired:** left on the fallbacks from the earlier round; no real adapter was in scope this round.

Demo and tests keep fakes; an empty demo uses the default empty UX state. Production leaves optimizer and GitHub seams empty so their `ClaudeOptimizer` and `RealGh` fallbacks remain active.

## Known limits

- Astra read-only tools are enforced only by the prompt (`src/codex/run.rs`), not by the sandbox.
- The Claude runner charges no usage on unparseable replies (`council/member.rs`).
