# Laya upstream notice

The code in `src/laya/` is ported from laya-rust (laya-onnx), upstream commit
`3a38cbe83f6ef556f3125fb0065fdc5a55d05792` (v0.4.31), licensed under Apache-2.0.
The upstream license is reproduced in `laya-Apache-2.0.txt`.

Changes: the `ort` crate is replaced by a libloading C-API binding, and the
`tokenizers` crate by a pure-Rust tokenizer.json reader. Upstream's Python
training scripts are used unchanged by the training bundle. The router,
shadow and curate modules are new.
