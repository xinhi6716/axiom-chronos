# Workshop module API v1

A module lives in `workshop/<name>/` and has a `manifest.json` with `name` (matching its directory), non-empty `version`, non-empty `entry` argv, optional `permissions` array, optional `max_parallel` (1–8, default 1), and `panel` with `title` and `description`. Unknown manifest fields are rejected.

Permissions: `files`, `memory`, `model:claude:<model>`, `model:codex:<model>` (use `*` for any model of that provider), `subprocess`, `network`, `env:<NAME>`, and `decision_router`. `subprocess`, `network`, and `env:<NAME>` are high-risk: starting a module requires human confirmation unless a prior approval for that module covers all its current high-risk permissions. Increasing that set requires confirmation again.

Workshop permissions are enforced at the API boundary and at spawn. This is not OS isolation: a module process can reach anything its user account can.

Modules exchange one JSON object per line over stdio. Requests are `{"v":1,"id":...,"method":"...","params":{...}}`. Replies carry `v`, the same `id`, and either `ok:true` with `result` or `ok:false` with `error:{kind,message}`. An unsupported or missing version returns `unsupported_version` with `error.supported:[1]`. Identity is supplied by the host channel; supplying `module` in the request or params is rejected.

Methods:

- `api.hello`: versions, module, permissions, max_parallel.
- `log.write` (`level`, `message`); `receipt.write` (`kind`, `body`) returns `sha256` of the serialized receipt before its hash field is added.
- `usage.get`: module-only usage totals.
- `memory.put` (`key`, `value`), `memory.get` and `memory.delete` (`key`), `memory.list`: require `memory`. Keys match `[a-z0-9._-]{1,128}`. Get returns `value` (null when absent), list returns `keys`.
- `file.read` (`path`), `file.write` (`path`, `content`), `file.list` (optional `path`, defaults to `.`): require `files`. Reads and writes are UTF-8; list returns `entries`.
- `model.call` (`provider`, `model`, `prompt`, optional positive `timeout_s`): requires matching model permission; returns `text` and `usage`. Calls wait for the module's `max_parallel` permit; Codex also waits for a global Codex slot, Claude does not.
- `process.spawn` (`argv`, positive `timeout_s`): requires `subprocess`; returns `exit_code`, `stdout`, `stderr` (each output limited to 64 KiB). The environment is cleared and rebuilt from baseline variables, granted `env:` variables, and module-specific paths.
- `net.request`: requires `network`, delegates params to the network service; the default service returns `unsupported`.
- `decision.route` (`question`, non-empty `options`): requires `decision_router`; returns `choice` and `reason`. The default router returns `unavailable`.

Errors include `unsupported_version`, `unknown_method`, `identity_spoof`, `permission_denied`, `path_escape`, `invalid_argument`, `cancelled`, and `failed`; service seams may also return `unsupported` or `unavailable`.

Per-module data is under `workshop-data/<name>/`: `files/`, `memory/memory.json`, `logs/module.log`, `receipts/receipts.jsonl`, `tmp/`, and host state and approval files. File paths are confined to `files/`. Receipts contain `ts`, host-supplied `module`, `kind`, `body`, and `sha256`. Usage records are in the engine home's `usage/extensions.jsonl`.

The host kill switch terminates the module and cancels its own waiting and in-flight calls; other modules are unaffected. Kill persists until revive. Cancelled calls release their permits and slots.

CLI integration should pair `BackendHeldSlots` with `CodexBackend`, because `AppServer::run_cancellable` acquires the global Codex slot itself. Use `GlobalSlots` instead with backends that do not acquire that slot. The app-server may wait for interrupt completion after cancellation.
