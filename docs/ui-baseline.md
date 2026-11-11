## Captures

| view | theme | size | file |
| --- | --- | --- | --- |
| run | dark | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/run-dark-1100x700.png |
| run | dark | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/run-dark-1920x1080.png |
| run | light | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/run-light-1100x700.png |
| run | light | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/run-light-1920x1080.png |
| wishes | dark | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/wishes-dark-1100x700.png |
| wishes | dark | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/wishes-dark-1920x1080.png |
| wishes | light | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/wishes-light-1100x700.png |
| wishes | light | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/wishes-light-1920x1080.png |
| memory | dark | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/memory-dark-1100x700.png |
| memory | dark | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/memory-dark-1920x1080.png |
| memory | light | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/memory-light-1100x700.png |
| memory | light | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/memory-light-1920x1080.png |
| usage | dark | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/usage-dark-1100x700.png |
| usage | dark | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/usage-dark-1920x1080.png |
| usage | light | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/usage-light-1100x700.png |
| usage | light | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/usage-light-1920x1080.png |
| settings | dark | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/settings-dark-1100x700.png |
| settings | dark | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/settings-dark-1920x1080.png |
| settings | light | 1100x700 | ~/.axiom/orchestrate/r6-ui2/captures/before/settings-light-1100x700.png |
| settings | light | 1920x1080 | ~/.axiom/orchestrate/r6-ui2/captures/before/settings-light-1920x1080.png |

Capture sizes denote the requested app size; PrintWindow includes the Windows frame, yielding 1116x708 and 1936x1088 PNGs respectively at 96 DPI. The demo Run is running, not blocked, so the needs-you state needs a separate blocked-state check.

## Weaknesses

- **W1** `run` `layout` — Run: needs-you card and checklist visible without scrolling is not established for a blocked run at the minimum size; the captured running demo cannot verify the card. PASS IF at 1100x700 both are fully visible with no scrolling. COST none.
- **W2** `run` `usability` — The Run stage notes and other labels use 11px text, below the design minimum of 12px. PASS IF all Run content text is at least 12px in both themes. COST none.
- **W3** `run` `interaction` — Resized Run column widths are not saved, so the user must repeat their adjustment after reopening. PASS IF adjusted column widths survive closing and reopening the app. COST none.
- **W4** `wishes` `usability` — At 1100x700 the running wish text is truncated while the wide capture shows it in full. PASS IF the complete wish can be read from the row without opening the run, including at 1100x700. COST none.
- **W5** `wishes` `effects` — Wish row corners use 6–8px radii instead of the Celestial Order maximum of 4px. PASS IF ordinary wish-row corners are at most 4px in both themes. COST none.
- **W6** `memory` `layout` — The list stays narrow on the wide screen and truncates the second entry subtitle while the detail pane has abundant unused width. PASS IF a wider window gives the list enough width to show that complete demo subtitle without clipping. COST none.
- **W7** `memory` `interaction` — Memory rows have no right-click menu for the actions shown in the detail pane. PASS IF right-clicking an entry exposes correction and deletion actions and deletion still asks for confirmation. COST none.
- **W8** `memory` `effects` — The selected list row and detail card use 6–8px corners instead of the 4px design limit. PASS IF those memory surfaces have corners no greater than 4px in both themes. COST none.
- **W9** `usage` `layout` — Model and role cost tables remain vertically stacked on a wide window, putting the role table below the first viewport. PASS IF at 1920x1080 the demo model and role tables are side by side and both table headers and first rows are visible without scrolling. COST none.
- **W10** `usage` `hierarchy` — At 1100x700 the first cost table starts below the viewport after the large context and warning panels. PASS IF at 1100x700 the model-cost table header and first demo row are visible together with the warning without scrolling. COST none.
- **W11** `settings` `layout` — The preferences column remains fixed at 400px when the window grows, leaving a narrow preferences card within large unused margins. PASS IF the preferences column expands with available content width at 1920x1080 while its controls remain legible at 1100x700. COST none.
- **W12** `settings` `usability` — There is no density choice in the preferences shown, so users cannot choose a more compact layout. PASS IF Settings offers a persistent density choice and changing it visibly adjusts panel spacing. COST none.
- **W13** `all` `effects` — The static backdrop lacks continuous animated atmosphere, but making it move would conflict with idle restraint. PASS IF a proposed animated backdrop is evaluated against an idle frame and CPU measurement before adoption. COST continuous idle CPU and GPU work, no model tokens. DROPPED continuous animation violates the idle design principle and adds no demonstrated usability benefit.

## Inspiration

unreachable
