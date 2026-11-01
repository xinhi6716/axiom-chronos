---
name: wish
description: Run a Chronos wish through the configured Rust bridge.
---

# Wish

Call the bridge `dispatch` tool with `project`, absolute `repo_root`, a unique `request_id`, and `action: {"type":"start","intent":"..."}`. Await the same call to its boundary; do not poll, start another core, or inject a continuation. Keep the request id and identical input for replay after transport loss. A lost host connection does not guarantee that the process survives.

At `needs_intent_review`, inspect the returned research artifact and decide explicitly with its digest and a reason. Do not pass automatically. For an existing run use `status`, `resume`, or `cancel` as appropriate. Unknown writes require reconciliation, not automatic redispatch. Report `ready_to_commit` as ready, not released. Exit zero means the request reached a boundary, not that work or installation completed.
