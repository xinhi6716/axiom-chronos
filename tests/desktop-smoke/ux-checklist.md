# UX checklist

## UX01 planning looks frozen
Scenario: planning
Element: run-heartbeat
After en: Last activity:
After zh-TW: 最後活動：
Before lacks: No last activity or run-live-stage indicator.

## UX02 blank between modules
Scenario: between_modules
Element: run-claude-activity
After en: Core · reviewing
After zh-TW: 主控 · 審查中
Before lacks: No Claude activity between modules.

## UX03 council invisible, no research identity
Scenario: council
Element: council-orbit
After en: Council · round
After zh-TW: 議會 · 第
Before lacks: No council orbit or research-emblem.

## UX04 silent max-turns stop
Scenario: max_turns
Element: notice-max-turns
After en: Stopped at the turn limit
After zh-TW: 已達回合上限而停止
Before lacks: No turn-limit notice.

## UX05 degraded council hidden
Scenario: degraded_council
Element: notice-degraded-council
After en: Council continued without
After zh-TW: 議會在缺少
Before lacks: No degraded council notice.

## UX06 unexplained scope failures
Scenario: scope_failure
Element: notice-scope-failure
After en: Outside the allowed paths
After zh-TW: 超出允許路徑
Before lacks: No explanation of scope failure.

## UX07 raw JSON events
Scenario: raw_events
Element: event-summary
After en: No visible recent-events text starts with {"
After zh-TW: No visible recent-events text starts with {"
Before lacks: Raw JSON events are painted unprocessed and no event-summary element exists.

## UX08 stale runs, no mark-read
Scenario: stale_runs
Element: run-mark-read
After en: Mark as read
After zh-TW: 標為已讀
Before lacks: No mark-read control.

## UX09 usage not per wish
Scenario: usage_per_wish
Element: usage-wish-picker
After en: All wishes
After zh-TW: 全部願望
Before lacks: No wish picker.

## UX10 who is running unclear
Scenario: who_running
Element: who-running
After en: Working now
After zh-TW: 正在工作
Before lacks: No working-now summary.

## UX11 closing stops work
Scenario: closing
Element: close-keeps-working
After en: keeps working in the background
After zh-TW: 在背景繼續
Before lacks: No background-work explanation.

## UX12 look-only screens
Scenario: unit_actions
Element: unit-retry
After en: Retry
After zh-TW: 重試
Before lacks: No retry or unit-waive control.
