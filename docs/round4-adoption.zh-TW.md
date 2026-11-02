# 第四輪參考資料採納

Codex-only 變體的同供應商 review 未採納；真實宿主驗收未驗證。舊七項的 manifest 本輪已讀，來源程式碼本輪未重審。

## DeepSeek
來源：E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/deepseek-ai--dsh-superagent-prototype，deepseek-ai/dsh-superagent-prototype，HEAD = 6337fc823ce20251c8de8085948165c81aaa701e（manifest tree id）。  
閱讀：承襲 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md；先記錄占用再執行。  
整合：claims.tsv 在送出 turn 前寫入 attempt id；僅同 id 可釋放，bridge 消失時保留 outcome_unknown／unknown <id>，不再派送；可用 orch release <unit> --attempt <id>。  
落點：claim src/orch/lifecycle.rs src/orch/store.rs src/orch/commands.rs src/bridge/codex_tool.rs tests/orch_claims.rs  
測試：tests/orch_claims.rs 檢查占用與釋放。  
排除：完整編排框架、UI、worktree 管理及 Codex checkpoint resume；只取占用機制，thread 仍是短暫的。  

## Orca
來源：E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/stablyai--orca，stablyai/orca，HEAD = 74bcbf0557efdd5ca1abb873d9b4611d03ef3866（manifest README blob id，未記錄 tree）。  
閱讀：承襲 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md；區隔新舊執行。  
整合：以 attempt id 隔離新舊占用；bridge 消失的 claim 為 outcome_unknown／unknown <id>，不再派送。  
落點：claim src/orch/lifecycle.rs src/orch/store.rs src/orch/commands.rs src/bridge/codex_tool.rs tests/orch_claims.rs  
測試：tests/orch_claims.rs 檢查 attempt 身分與未知結果。  
排除：完整編排框架、UI、worktree 管理及 Codex checkpoint resume；只取執行隔離，thread 仍是短暫的。  

## Stop That Shit
來源：E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/lennney--stop-that-shit，lennney/stop-that-shit，HEAD = e4fe5296f6917849f7eec0860d6a1b68bc0292de（manifest tree id）。  
閱讀：承襲 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md；占用的擁有權。  
整合：claims.tsv 在送出 turn 前記錄 attempt id，只允許同 id 釋放；未知結果保留占用且不再派送，可明確執行 orch release <unit> --attempt <id>。  
落點：claim src/orch/lifecycle.rs src/orch/store.rs src/orch/commands.rs src/bridge/codex_tool.rs tests/orch_claims.rs  
測試：tests/orch_claims.rs 檢查占用所有權。  
排除：完整編排框架、UI、worktree 管理及 Codex checkpoint resume；只取占用所有權，thread 仍是短暫的。  

## Pstack
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/pstack（cursor/plugins Pstack），HEAD ae5650f。  
閱讀：本輪實讀 pstack/skills/poteto-mode/playbooks/orchestrate.md、principle-sequence-verifiable-units、principle-make-operations-idempotent；舊七項承襲來源見 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md。  
整合：review 只對 VERIFY 執行時且仍在磁碟上的快照有效；不符為 stale_review，verified 要求非空且相等的 commits。  
落點：verdict src/orch/evidence.rs tests/verdict.rs tests/orch/evidence_cases.rs  
測試：tests/verdict.rs、tests/orch/evidence_cases.rs 檢查 verdict 與證據。  
排除：orchestrate head／verdict 規則以外的 playbook 文字；該規則已是派送設計基礎。  

## Codebase Memory
來源：E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/DeusData--codebase-memory-mcp，DeusData/codebase-memory-mcp，HEAD = f29e74f276af08456ba80c3b8e21f466bd00765b（manifest tree id）。  
閱讀：承襲 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md；來源綁定。  
整合：verified unit 的 memory_record 必須有非空 files；project、unit 與檔案雜湊相同時回傳既有 id（duplicate）。  
落點：memory src/memory/mod.rs engine/agents/memory-keeper.md tests/memory.rs  
測試：tests/memory.rs 檢查記錄與去重。  
排除：圖索引、MCP server、schema 變更；現有記錄與雜湊已涵蓋此處需求。  

## Trail-Code
來源：E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/trailhq--Trail-Code，trailhq/Trail-Code，HEAD = a44da3dfb5894c8e316e320526041ef8e5ddbb8c（manifest tree id）。  
閱讀：承襲 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md；內容新鮮度。  
整合：verified unit 的 memory_record 必須有非空 files；project、unit 與檔案雜湊相同時回傳既有 id（duplicate）。  
落點：memory src/memory/mod.rs engine/agents/memory-keeper.md tests/memory.rs  
測試：tests/memory.rs 檢查檔案綁定與去重。  
排除：圖索引、MCP server、schema 變更；只取檔案內容綁定。  

## Agency Agents
來源：E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/msitarzewski--agency-agents，msitarzewski/agency-agents，HEAD = b37b5bb33b78f2c1d6567d98e25ca818e3259219（manifest tree id）。  
閱讀：承襲 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md；交接與實證。  
整合：bridge 的真實 VERIFY 輸出保存在 reports/<unit>.verify.txt，含 BRIDGE VERIFY 行；HANDOFF 包含 RESULT、PATHS、COMMANDS、OPEN、EVIDENCE 五欄。  
落點：claim src/orch/lifecycle.rs src/bridge/codex_tool.rs engine/DISPATCH.md engine/agents/dispatcher.md tests/orch_claims.rs  
測試：tests/orch_claims.rs 檢查 claim 與交接證據。  
排除：persona 目錄與角色提示詞；只取精簡交接與證據。  

## HERO
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/wanshuiyin-HERO-Anti-OverDefense，HEAD 92686ca。  
閱讀：本輪實讀 RULES.md 的四類問題與範圍限制、skills/goal-pulse/SKILL.md、scripts/goal_pulse.py 的 tick，並搜尋 README；舊七項承襲來源見 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md。  
整合：memory keeper 在前景執行，active goal 下拒絕背景呼叫；Stop 續跑只帶 session、120 字元 wish 前綴與下一步；creative mode 僅在 ruling 選出候選時呼叫 architect。  
落點：wish-flow src/hook/mod.rs engine/skills/wish/SKILL.md tests/hook.rs tests/engine/text_cases.rs  
測試：tests/hook.rs、tests/engine/text_cases.rs 檢查 hook 與流程文字。  
排除：goal-pulse 自動 done 與 tick 上限；goal 持續到 done 或 blocked，不設上限、不自動完成。  

## ARIS
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/wanshuiyin-Auto-claude-code-research-in-sleep，HEAD c485faa。  
閱讀：本輪實讀 README 開頭、research-pipeline 的 routing／checkpoint／resume、auto-review-loop 的 reviewer／resume、experiment-plan Phases 0–4、shared-references/resumable-runs.md；舊七項承襲來源見 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md。  
整合：實證問題的計畫列出 claim、替代解釋、baseline、開發／驗收資料隔離、執行前固定門檻與可反駁反例；第二次 review 檢查洩漏及移動門檻。  
落點：research engine/skills/research/SKILL.md engine/agents/critic.md tests/research_text.rs  
測試：tests/research_text.rs 檢查研究流程文字。  
排除：無人值守過夜 pipeline、auto-review-loop 輪數上限及額外資源檔；只取研究與審查約束。  

## ECC
來源：E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/affaan-m--ECC，affaan-m/ECC，HEAD = b38be6a7f6350964d59328b52f0624e1d1d552d1（manifest tree id）。  
閱讀：承襲 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md；資源閉包與原生驗證邊界。  
整合：安裝的 engine 資源相對引用均可解析，並有負控制；launch／drive／observe／cleanup 真實宿主清單區分 fake 回歸與真實驗收。  
落點：install tests/installed_assets.rs docs/bridge.md  
測試：tests/installed_assets.rs 檢查引用及負控制；真實宿主清單未驗證。  
排除：ECC 的 hooks／rules 目錄；只取資源閉包及驗收邊界。  

## Axiom Atropos
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/xinhi6716-axiom-atropos，HEAD 1f39b07。  
閱讀：本輪實讀 create-verification-skill、principle-guard-the-context-window、principle-subtract-before-you-add、architect 前段、research-suite 與 routing 片段、benchmark-checklist、scientific-critical-thinking，並搜尋 principle-test-behavior-not-implementation；舊七項承襲來源見 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md。  
整合：安裝的 engine 資源相對引用均可解析並有負控制；launch／drive／observe／cleanup 真實宿主清單區分 fake 回歸與真實驗收。  
落點：install tests/installed_assets.rs docs/bridge.md  
測試：tests/installed_assets.rs 檢查引用及負控制；真實宿主清單未驗證。  
排除：整套 Atropos skill library；只取驗證與資源邊界。  

## Orkas
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/Orkas-AI-Orkas，HEAD dce78ad。  
閱讀：本輪實讀 README 開頭、src/core-agent/src/agent/progress-evidence.ts、loop-guards.ts 前段、progress-evidence.test.ts 案例，並搜尋 progress-runner.test.ts；舊七項承襲來源見 E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex/docs/codex-rebuild.zh-TW.md。  
整合：活動與結果分離；呼叫帶 progressToken 時，真實 Codex／agy 事件成為 MCP notifications/progress，不使用計時 heartbeat，progress 不改變狀態。  
落點：progress src/base/progress.rs src/bridge/server.rs src/codex/run.rs src/agy/print.rs tests/progress.rs  
測試：tests/progress.rs 檢查進度；真實 Claude Code 宿主搭配通知的 idle-timeout 行為未驗證。  
排除：TypeScript loop guards 與 runtime；只取活動證據與結果分離。  
