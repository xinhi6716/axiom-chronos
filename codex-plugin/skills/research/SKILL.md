---
name: research
description: Research a question through the configured Chronos Rust bridge.
---

# Research

Use the `wish` skill dispatch protocol with the research question as intent. Await the boundary and return at `needs_intent_review` with the research artifact and its digest; do not automatically decide or start engineering. Reuse the same request id and input after transport loss. Do not poll or launch another core.
