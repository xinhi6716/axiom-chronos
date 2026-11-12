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
