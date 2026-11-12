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

Result: measured; frozen acceptance not met

<!-- AB_RESULT -->

### Measured result

72 isolated `claude -p` sessions were run (12 scenarios x 2 arms x 3 runs); the reports are `target/graph-ab/report.json` and `target/graph-ab/report.md`. Status: measured. The median across scenarios of `1 - median(B) / median(A)` for Claude processed input was **-24.08%**, so graph briefs (B) cost more than today's briefs (A). The frozen acceptance of at least 10% saving and zero regressions was **not met**; thresholds were not lowered and the batch was not re-run. The report's `thresholds_met` is `null`; this conclusion follows from the measured saving and regressions.

Claude classes, summed over the 36 runs of each arm (unknown count 0 for every class):

| class | A | B |
| --- | ---: | ---: |
| input | 170 | 198 |
| cache_write | 665991 | 661123 |
| cache_read | 3063612 | 3654546 |
| output | 32025 | 26823 |

Codex input, cache_write, cache_read and output are all **unknown** for all 36 runs in each arm, not zero. Budget overruns: none. Wrong certain edges: **unknown** (`wrong_certain_edges` is `null` in the report).

### Per-scenario regressions

All 12 scenarios have an evidence regression: the B brief lacks the scenario's main source file. Eleven also miss a required finding; s08 is the only exception. There are no condition regressions.

| Scenario | Missing source file | Missing finding |
| --- | --- | --- |
| s01 | `src/inventory.rs` | Reservation checks stock before decrementing it |
| s02 | `src/rainfall.py` | The mean excludes missing observations |
| s03 | `src/checkout.tsx` | Checkout checks expiry before discounting |
| s04 | `src/air_quality.jsx` | Calibration is applied before aggregation |
| s05 | `src/water_sampling.rs` | Milliliters are converted to liters |
| s06 | `src/scheduler.py` | Jobs are sorted by priority |
| s07 | `src/survey.tsx` | Scoring counts only answered items |
| s08 | `src/notifications.jsx` | None |
| s09 | `src/rate_limit.rs` | Retry is allowed below three attempts |
| s10 | `src/catalog.py` | Search titles are lowercased |
| s11 | `src/shipping.tsx` | Heavy shipments use a higher rate |
| s12 | `src/sessions.jsx` | Session expiry uses an inclusive comparison |

### Payload bytes (not tokens)

| Scenario | A bytes | B bytes |
| --- | ---: | ---: |
| s01 | 2626 | 540 |
| s02 | 2730 | 522 |
| s03 | 2570 | 527 |
| s04 | 2734 | 523 |
| s05 | 2655 | 518 |
| s06 | 2507 | 529 |
| s07 | 2734 | 587 |
| s08 | 2523 | 913 |
| s09 | 2473 | 521 |
| s10 | 2457 | 600 |
| s11 | 2514 | 602 |
| s12 | 2468 | 535 |

B briefs are smaller in bytes yet used more Claude processed input. Bytes are not token savings.

### Cause

The diagnosis used code inspection and `axiom graph explore` on the s01 fixture; the full live B brief was not rebuilt.

- The B brief is the constraints block plus the `graph_explore` packet (`src/graph/measure.rs:198-219`). The packet carries no source text, only key/at/summary bundles, so A's embedded source file is absent unless a bundle names the file.
- The explore query finds no seeds. For a non-fallback query, `src/graph/query.rs:152-191` matches the whole string: exact equality on label, key or file path (line 166), followed by one FTS phrase `"<whole query>"` (line 183) over a trigram index on label and summary (`src/graph/schema.sql:12`). Code node labels are identifiers such as `reserve` and paths, so manifest prose such as s01's `reservation stock` matches nothing. On the s01 fixture, `axiom graph explore --view code` returns `"seeds": []` for both `reservation stock` and `reservation`, but finds `sym:src/inventory.rs#function:reserve` for `reserve`.
- The recorded memory entries are not matched either: the contiguous phrase `reservation stock` is in neither the entry title nor its summary. B briefs measure 518-913 bytes, the constraints block plus an empty packet.
- Fallback queries for short CJK or short words (`src/graph/query.rs:95-114, 155-164`) use LIKE over title, summary and key, which do not contain the CJK query terms for code nodes either (s02, s04).
- The leading hypothesis for the extra cost is that, with an empty packet, the agent must locate and read the source itself, adding turns and cache reads. This is not confirmed.

Status: cause confirmed for the empty packet (no seeds for multi-word prose queries); the link between the empty packet and the higher token cost is not yet confirmed. A fix to query seeding (per-word matching and a code snippet in the packet) is needed, followed by a new live A/B run; no fix has been completed.

## Honesty and limitations

The resolution holdout was seen while tuning the resolver, so its scores are **not independent**: Rust 0.75, Python/TS/JS 0.875, and zero wrong certain edges. They are not an independent measure of resolver performance. tree-sitter is unavailable offline, so Rust uses syn and Python/TS/JS use native hand-written lexers.
