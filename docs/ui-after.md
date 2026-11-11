# UI after

## Captures

| name | file |
| --- | --- |
| run-dark-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/run-dark-1100x700.png |
| run-dark-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/run-dark-1920x1080.png |
| run-light-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/run-light-1100x700.png |
| run-light-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/run-light-1920x1080.png |
| wishes-dark-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/wishes-dark-1100x700.png |
| wishes-dark-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/wishes-dark-1920x1080.png |
| wishes-light-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/wishes-light-1100x700.png |
| wishes-light-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/wishes-light-1920x1080.png |
| memory-dark-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/memory-dark-1100x700.png |
| memory-dark-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/memory-dark-1920x1080.png |
| memory-light-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/memory-light-1100x700.png |
| memory-light-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/memory-light-1920x1080.png |
| usage-dark-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/usage-dark-1100x700.png |
| usage-dark-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/usage-dark-1920x1080.png |
| usage-light-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/usage-light-1100x700.png |
| usage-light-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/usage-light-1920x1080.png |
| settings-dark-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/settings-dark-1100x700.png |
| settings-dark-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/settings-dark-1920x1080.png |
| settings-light-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/settings-light-1100x700.png |
| settings-light-1920x1080.png | ~/.axiom/orchestrate/r6-ui2/captures/after/settings-light-1920x1080.png |
| blocked-1100x700.png | ~/.axiom/orchestrate/r6-ui2/captures/after/blocked-1100x700.png |
| focus-graph.png | ~/.axiom/orchestrate/r6-ui2/captures/after/focus-graph.png |
| focus-checklist.png | ~/.axiom/orchestrate/r6-ui2/captures/after/focus-checklist.png |
| mini-top-off.png | ~/.axiom/orchestrate/r6-ui2/captures/after/mini-top-off.png |
| dense.png | ~/.axiom/orchestrate/r6-ui2/captures/after/dense.png |
| at-rest.png | ~/.axiom/orchestrate/r6-ui2/captures/after/at-rest.png |
| feed-error.png | ~/.axiom/orchestrate/r6-ui2/captures/after/feed-error.png |
| stale.png | ~/.axiom/orchestrate/r6-ui2/captures/after/stale.png |
| quota-low.png | ~/.axiom/orchestrate/r6-ui2/captures/after/quota-low.png |
| quota-out.png | ~/.axiom/orchestrate/r6-ui2/captures/after/quota-out.png |
| quota-unknown.png | ~/.axiom/orchestrate/r6-ui2/captures/after/quota-unknown.png |

## Verdicts

W1 PASS blocked-1100x700.png - needs-you card and checklist both whole, no scrolling
W2 PASS run-dark-1100x700.png - run text at least 12px, held by tests/desktop_style.rs
W3 PASS run-dark-1100x700.png - column widths persist, walk test column_widths_survive_a_restart
W4 PASS wishes-dark-1100x700.png - running wish title wraps to two lines with no ellipsis, folder-only subtitle
W5 PASS wishes-dark-1100x700.png - row corners at most 4px, held by tests/desktop_style.rs
W6 PASS memory-dark-1920x1080.png - second subtitle shown whole
W7 PASS memory-dark-1100x700.png - right-click and shift-f10 menu with correction and deletion, deletion confirms (walk tests)
W8 PASS memory-dark-1100x700.png - corners at most 4px, held by tests/desktop_style.rs
W9 PASS usage-dark-1920x1080.png - model and role tables side by side, headers and first rows visible
W10 PASS usage-dark-1100x700.png - warnings fold to one line and the model-cost header and first row are visible at 1100x700 without scrolling
W11 PASS settings-dark-1920x1080.png - preferences column fills its half at 1920x1080 and stays legible at 1100x700
W12 PASS dense.png - density choice in Settings, persists, row heights change (walk tests)
W13 DROPPED run-dark-1100x700.png - continuous animation violates idle restraint

IDLE CPU 0.21% over 30 s

This is one core percent, measured with `--demo --scene at-rest --offscreen`: Process TotalProcessorTime delta 0.0625 s over 30 s.

## Not verified

- Native minimise and restore of the main window when Mini opens.
- Mini always-on-top on and off (no capture or test signal for a PopUp window).
- Delete-confirmation title text (plain text not in snapshots).
- Selection glow and focus-ring painting (no shadow in snapshots).
- 60 s quota repaint.
- Startup with a saved Mini.
- Tooltip rendering.
- Real-engine and native-window behaviour.

## Record keeping

The checklist dispatcher saved its verdict files with findings emptied (`checklist-1.verdict.json` has findings empty), and several later verdicts were condensed rather than saved verbatim. `checklist-2.verdict.json` was restored with its 4 minor findings: thin scanners (the raw px scanner with an empty allowlist misses margin methods and the render body; the shift-f10 test only scans source); menu action resolution and filter save and restore are untested; and `open_cursor_menu` scrolled-out and unfocused rules are untested.

The audit found and fixed real defects through sol units: alt-arrows gave no focus on a fresh Run screen; wishes Escape/Up/reorder menu handling; shift-f10 bounds order in wishes and memory; and four decision-19 radius violations.
