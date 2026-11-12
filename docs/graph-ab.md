# Graph brief A/B

## Protocol

Compare today's briefs (A) with graph briefs (B) across 12 frozen scenarios, with three runs per arm per scenario. Each run uses an isolated `claude -p` session; this is never a wish. Set `AXIOM_AB_CLAUDE` to a JSON argv such as `["claude","-p"]`, then run:

```sh
cargo test --locked --offline --test graph_ab_live -- --ignored
```

The harness writes `target/graph-ab/report.json` and `target/graph-ab/report.md`. If the variable is unset, it produces a not-measured report without running Claude.

## Metrics

Claude processed input = input + cache_write + cache_read. For each scenario, calculate `1 - median(B) / median(A)` from its three runs per arm; the saving is the median of those scenario savings. The saving threshold is 10%. Other acceptance checks cover evidence, conditions, major findings, budget overruns and wrong certain edges; the threshold is not a result.

List Claude's input, cache-write, cache-read and output classes separately, and list Codex usage separately again. Missing usage is **unknown**, never zero. Payload byte deltas belong in a separate table and are never token savings; the JSON report records A and B byte counts per scenario.

## Current status

Result: not measured yet

<!-- AB_RESULT -->

## Honesty and limitations

The resolution holdout was seen while tuning the resolver, so its scores are **not independent**: Rust 0.75, Python/TS/JS 0.875, and zero wrong certain edges. They are not an independent measure of resolver performance. tree-sitter is unavailable offline, so Rust uses syn and Python/TS/JS use native hand-written lexers.
