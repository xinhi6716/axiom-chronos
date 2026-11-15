# CodeGraph attribution

This port follows the upstream CodeGraph design, colbymchenry/codegraph@f72c9d2efb14454ff4f9fd7b8460abd82f19a287 (MIT). The source was not available offline, so no upstream text was copied verbatim.

- `model.rs`, `rust.rs`, and `script/` follow upstream `src/extraction/*`.
- `index/` follows upstream `src/resolution/*` and `src/db/schema.sql`.

tree-sitter is missing offline, so Rust uses syn and Python, TS/TSX, and JS/JSX use a hand-written lexer. The extractors here are placeholders.

See [LICENSE-codegraph.txt](LICENSE-codegraph.txt).
