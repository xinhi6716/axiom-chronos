# 第五輪參考資料採納

## DeepSeek
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/deepseek-ai-dsh-superagent-prototype HEAD 6337fc823ce20251c8de8085948165c81aaa701e
閱讀：本輪實讀 src/core/assignments.ts；docs/003-execution.md、docs/007-architecture.md、src/core/engine.ts、src/app/runtime.ts 的相關區段；tests/runtime-concurrency.spec.ts 僅案例搜尋。
未讀：engine.ts、runtime.ts 僅讀相關區段；其餘 ticket／thread／UI 系統未完整閱讀。
採納：assignments.ts 區分活動與已結束的指派；標準版只採用依賴單元必須已註冊且已驗證的判斷，不採 ticket 系統。
落點：orch-gates src/orch/lifecycle.rs
問題：第四輪 20261102-193323-wish.jsonl :736 因 verdict 依賴未完成而 not_ready，:753–759 複製 brief 改成 build track 後成功派工，繞過模組依賴；舊 lifecycle 把未知 track 當無依賴。
測試：tests/orch/lifecycle_cases.rs::dependencies_require_verified_unit tests/orch/lifecycle_cases.rs::tracks_dependencies_and_auto_accept
不採納：ticket／thread／訊息總線與工作區服務，標準版已有 claim 與 store，另建一套是平行系統；結果未知 claim（R5-08）因日誌沒有觀察到失敗而捨棄。

## Orca
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/stablyai-orca HEAD 8cd6a7e5ee77c4ed04e78a3e924f5fc83a366bbf
閱讀：本輪實讀 skill-guides/orchestration/references/recovery-and-cleanup.md；src/cli/orchestration-mutation-recovery.ts；src/main/runtime/orchestration/dispatch-message-binding.ts、dispatch-failure-idempotency.test.ts；db/worker-dispatch/worker-dispatch-start.ts 的啟動交易區段。
未讀：UI、daemon、PTY、遠端工作區及其餘測試未完整閱讀。
採納：dispatch-message-binding 把結果綁到發出它的派工；標準版要求審查結果指名同一單元，審查者為 chronos:reviewer。
落點：orch-gates src/orch/commands.rs
問題：第四輪 :685 的 Claude reviewer 只審 proc，:701 寫出 proc-review.json，:723 卻套到 verdict-fix 並標成 Gemini，:724 回報 verified。
測試：tests/orch/lifecycle_cases.rs::review_is_bound_to_unit_and_reviewer
不採納：not_sent／outcome_unknown 恢復流程、資料庫交易與 daemon；日誌沒有已送出工作被重派的事故，標準版不為此增加狀態。

## Codebase Memory
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/DeusData-codebase-memory-mcp HEAD a5b248f47272413189746589b9a2c551c6b6c5ec
閱讀：本輪實讀 docs/MEASURING_SAVINGS.md、EVALUATION_PLAN.md 相關區段；src/pipeline/pipeline_incremental.c 的增量閉包、manifest／發布檢查；src/mcp/compact_out.h、compact_out.c；mcp.c 的輸出格式、search_graph 與 get_code_snippet 相關區段。
未讀：全部語言解析器、圖查詢／儲存實作及完整測試未讀。
採納：只採增量、只讀有變動且有效的來源：memory-keeper 先列出 verified 單元，再只讀這些單元的 report 與仍存在的變更檔。
落點：engine-text engine/agents/memory-keeper.md
問題：第四輪 84 次 Read 中 59 次來自 memory-keeper，同輪沒有 memory_search／memory_get 呼叫，舊指示先讀每個單元的 brief、report、SCOPE。
測試：tests/engine_text.rs::memory_reads_verified_units_first
不採納：完整程式圖與索引；日誌的閱讀成本在記憶整理，不在程式導覽，沒有獨立 token／品質比較證明可抵銷建索引與維護成本；brief 與審查本來需要完整檔案文字。

## Agency Agents
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/msitarzewski-agency-agents HEAD b37b5bb33b78f2c1d6567d98e25ca818e3259219
閱讀：本輪實讀 strategy/coordination/handoff-templates.md；testing/testing-reality-checker.md、testing-evidence-collector.md 的核心規則；scripts/check-runbooks.sh、lint-agents.sh 相關實作。
未讀：全部角色與 runbook 未讀。
採納：handoff-templates 固定交接欄位與 reality-checker 須有證據、允許零問題；reviewer 一次審一個完整單元（單元 id、全部檔案含測試、契約），回傳 JSON 寫入 unit。
落點：engine-text engine/agents/reviewer.md
問題：第四輪共 27 次 review_code：14 次完成、5 次 too_large、3 次 900 秒逾時、5 次取消，逐檔切碎審查。
測試：tests/engine_text.rs::contract_sentences_are_embedded tests/engine_text.rs::reviewer_and_critic_use_claude
不採納：角色大集合與強制多輪審查；標準版只有固定的幾個角色，多輪審查會增加無效 loop。

## Trail-Code
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/trailhq-Trail-Code HEAD a44da3dfb5894c8e316e320526041ef8e5ddbb8c
閱讀：本輪實讀 README 相關說明；src/ai/summarize.ts、src/trace/seen-cache.ts、src/ask/file-selection.ts；src/context/build.ts、src/graph/extract-cache.ts、build.ts、enrich.ts、src/ask/graphrank.ts 的快取／選檔區段；test/ask-file-selection.test.ts 相關案例。
未讀：完整索引、圖排序迭代、其他 AI 流程與全部測試未讀。
採納：依內容雜湊重用既有結果；標準版已有 memory_search／memory_get 與來源雜湊，只在 dispatcher 規定寫 brief 之前先呼叫它們。
落點：engine-text engine/agents/dispatcher.md
問題：第四輪沒有任何 memory_search／memory_get 呼叫，已入庫的內容沒被重用。
測試：tests/engine_text.rs::contract_sentences_are_embedded
不採納：全庫 LLM 摘要快取與 PageRank 選檔；沒有 token／品質比較支持，且摘要不能取代完整程式審查。

## Stop That Shit
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/lennney-stop-that-shit HEAD e4fe5296f6917849f7eec0860d6a1b68bc0292de
閱讀：本輪實讀 ARCHITECTURE.md、skills/stop-that-shit/SKILL.md 的核心原則；src/controller.cjs 相關區段；src/state.cjs 的 owner lock／死亡確認／釋放實作；test/session-lock.test.cjs 相關案例；docs/agents/domain.md。
未讀：完整 controller／contracts 未讀。
採納：以 owner 身分而非鎖齡判定釋放；accept_if_ready 使用行程內依模組的鎖，同一模組同時完成只驗收一次，並先確認沒有單元持有 claim。
落點：orch-gates src/orch/lifecycle.rs
問題：這是原始碼可達的競態，第四輪日誌沒有實際重複驗收；第四輪有 2 個 ACCEPT 回合由模型手動執行（人工分類）。
測試：tests/orch/lifecycle_cases.rs::tracks_dependencies_and_auto_accept
不採納：全域合約、預算控制與跨行程鎖；標準版是單一 binary，沒有觀察到跨行程競爭。

## ECC
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/affaan-m-ECC HEAD b38be6a7f6350964d59328b52f0624e1d1d552d1
閱讀：本輪實讀 scripts/lib/context-retrieval.js、scripts/lib/install/link-rewrite.js、scripts/hooks/evaluate-session.js 的核心區段；skills/iterative-retrieval/SKILL.md、skills/eval-harness/SKILL.md 的流程與評估原則。
未讀：全部 skills／hooks／安裝器未讀。
採納：link-rewrite.js 以實際檔案索引核對連結；installed_assets 測試先用 serde_json 解碼 JSON 再掃字串，並以精確大小寫的實際安裝檔案集合核對。
落點：install tests/installed_assets.rs
問題：第一輪 20261029-234647-wish.jsonl:162 曾把 JSON 跳脫後的 hooks/goal.py 路徑判錯。
測試：tests/installed_assets.rs::checker_decodes_json_strings_and_reports_malformed_json
不採納：整套 hooks／skills 與安裝器；標準版是單一 binary，保留官方 Codex 功能，不載入外部框架。

## Orkas
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/Orkas-AI-Orkas HEAD dce78adb2a2ffdfbf9159cc89352e4335940bc64
閱讀：本輪實讀 src/core-agent/src/agent/loop-guards.ts 全部 322 行。
未讀：progress-evidence.ts 與完整整合測試未讀。
採納：loop-guards 把重複查詢視為沒有進展；單元完成後由 Rust 一次執行 ACCEPT 並在同一回覆回傳，模型不必重複查狀態。
落點：unit-run src/bridge/codex_tool.rs
問題：第四輪人工分類出 9 個純狀態查詢回合（人工分類的開發估計，見量測）。
測試：tests/unit_run.rs::waits_for_all_module_units tests/unit_run.rs::auto_accept_and_verify_failure
不採納：固定硬停止門檻與全域 loop 計數；會誤殺長時間的正常工作，改以實際進展與既有狀態判斷。

## HERO
來源：E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/wanshuiyin-HERO-Anti-OverDefense HEAD 92686ca5c7dc9f4852cd323e2852219b7e86a8ea
閱讀：本輪實讀 RULES.md 英文核心規則；cases/quant-research-audit-loop.md；skills/goal-pulse/scripts/goal_pulse.py 的開頭與函式盤點。
未讀：其他案例、翻譯版本及 goal-pulse 的完整執行流程未讀。
採納：結論要對得上需求、不為防禦多跑一輪；wish 技能的創造力無候選路徑只跳過 architect 與 dispatcher，仍走記憶、改寫、報告、結案，不再迴圈。
落點：wish-flow engine/skills/wish/SKILL.md
問題：SKILL.md 第 3 步「skip to the close」與第 4 步「proceed to step 5」互相矛盾，可被解讀為直接結案。
測試：tests/wish_flow.rs::creative_no_candidate_runs_remaining_steps
不採納：週期性 goal-pulse daemon 與另開的審查員檢查清單階段；標準版 goal 已做到 done 或 blocked，不新增常駐程序或審查階段。

## HarnessMM
來源：harnessmm-2610.32813/paper.txt
閱讀：本輪實讀 paper.txt 主文方法、結果與限制，以及附錄 B、C.3–C.5、D、E、K／L 的相關段落；重點核對回呼移除條件、完整成本、資料隔離、消融與品質退步案例。
未讀：paper.txt 未逐行讀完 2493 行；部分附錄、完整參考文獻及圖表細節未讀，paper.pdf 未做視覺核對；沒有取得或重現論文程式與實驗。
採納：只做兩個固定轉換，不做通用決策圖：(1) 單元 VERIFY 通過且模組全部單元通過時，bridge 在同一個 codex_run 回覆內執行 ACCEPT，先重查 claim、停止與追加狀態；(2) orch review 的回覆附帶 ready／waiting 摘要。
落點：unit-run src/bridge/codex_tool.rs src/goal/mod.rs
問題：第四輪 167 個可見、去重 assistant message（Sonnet 91、Opus 63、Haiku 13）中，人工辨識 14 個為執行既定下一步（2 個 ACCEPT、3 個 review 記帳、9 個純狀態查詢），約 8.4%；這是人工分類的開發估計，不是已實現的節省。
測試：tests/unit_run.rs::auto_accept_and_verify_failure tests/unit_run.rs::goal_holds tests/unit_run.rs::accept_failure_does_not_fail_run
不採納：通用決策圖引擎、論文的 RSI 管線與路由模型、Rust 內建 Claude 呼叫器；不引用論文的約 30% 或 96k→12k，因為論文結果混有其他 harness 變更。

## 第四輪機制重判
- 持久化 claim：保留，DeepSeek／Stop 的原始碼支持以身分釋放；R5-08 因日誌無失敗而不增加狀態。
- 進度通知：保留，第四輪日誌有 478 筆 tool_progress，未發現 MCP 閒置逾時；本輪日誌 tool_progress 也未見逾時。
- 快照綁定審查：保留，是 R5-06 單元綁定的基礎；已知缺口：單元 SCOPE 含已刪除路徑時快照為空，審查會被判為過期（本輪 bridge-tools 單元遇到，需重新派工才能審查），尚未修，列為未解項；不加快照一致性層。
- 記憶去重：保留，本輪不加 project_dir 去重與新鮮度機制（無事故）。
- 前景 memory-keeper：保留，但讀取順序改為 verified 單元優先（R5-10）。
- 研究紀律：保留，critic 與 research 技能的資料外洩與門檻檢查原樣。
- 行程樹診斷：保留，沒有證據顯示過度或失效，移除也無節省。
- Gemini 審查：移除，第四輪 27 次 review_code 只有 14 次完成（5 too_large、3 逾時、5 取消）且用掉 78% 週額度；審查改由 chronos:reviewer 與 chronos:critic（Claude）負責，Gemini 只保留白話改寫（R5-04）。

## 全面優化
- Token 效率：R5-09 與 R5-10。前：計數 Claude 回合中執行既定下一步者（14/167，人工分類）與 memory-keeper 的 Read 數（59）；後：同口徑重數第五輪日誌。
- 速度：R5-04。前：第四輪 review_code 3 次 900 秒逾時、5 次 too_large、5 次取消；後：review_code 呼叫數為 0，審查改為一單元一次。
- 成功率：R5-05。前：第四輪 SCOPE 註記造成 4 次拒絕、單元模式混入 brief 造成 6 次 invalid_argument、首批 6 個 VERIFY 回報 CARGO_HOME is not recognized；後：以 is_error 數與 invalid_argument 數重數第五輪日誌。
- 返工：R5-05 與 R5-07。前：第四輪 VERIFY shell 差異導致重新派工、依賴未完成時用 build track 繞過；後：未知 track 與未註冊依賴被拒絕，重派數以 attempt 大於 1 的單元數計；不採 VERIFY 重跑捷徑。
- 子代理調度：R5-04 與 R5-07。前：review_code 逐檔切碎；後：reviewer 一單元一次，module track 受 modules.md 約束。
- 忙等卡死：本輪不動，第四輪與本輪日誌皆未發現 MCP 閒置逾時，R4-01 進度通知保留；若真機出現逾時按 bug 修，不用環境變數繞過。
- 穩健性：R5-01 與 R5-02。前：wish 技能無候選路徑可能直接結案、JSON 跳脫路徑判錯；後：兩者各有帶負控制的測試。
- 狀態機與權限：R5-06 與 R5-07。前：proc 的審查被套到 verdict-fix 並回報 verified；後：審查必須指名同一單元與 chronos:reviewer，錯綁被拒絕。
- 研究計畫：本輪不動，第四輪的研究紀律已有測試，日誌沒有新的研究失敗證據。
- RSI：本輪不動，HarnessMM 的 RSI 管線沒有 Chronos 內的證據支持，且論文結果混有其他 harness 變更。
- 記憶：R5-10。前：59/84 次 Read 來自 memory-keeper；後：memory-keeper 只讀 verified 單元，以同口徑重數。
- 驗證品質：R5-04 與 R5-06。前：Gemini 審查 27 次呼叫只有 14 次完成；後：Claude reviewer 一單元一次、結果綁定單元。
- 版本一致性：本輪不動，版本維持 0.2.0-beta.1，同提交的 Windows／Linux CI 為交付時核對項，不改 CI。
- 可觀察性：R5-09。前：模型需另外呼叫 orch 才知道 ACCEPT 與 ready 狀態；後：codex_run 回覆帶 accept，orch review 回覆帶 ready／waiting。
- 維護成本：R5-04。前：bridge 有 review_code、agy 審查實作、second_review 與 fallback；後：這些程式與測試已刪除，舊 stage 資料仍可讀。

## 量測
第四輪來源：C:/Users/gppup/.axiom/logs/20261102-193323-wish.jsonl；第五輪來源：C:/Users/gppup/.axiom/logs/20261103-004231-wish.jsonl，撰寫本單元時最新的 *-wish.jsonl，截至 2026-11-02T17:51:35Z，日誌未結束且執行仍在進行。
這些數字均是已讀日誌的開發證據，不是獨立驗收集；14/167 是人工分類的開發估計，不是已實現的節省。論文的 30% 與 96k→12k 不宣稱適用於 Chronos。
口徑：is_error 以 tool_result 的 is_error 為真計數（含取消）；review_code 以該工具呼叫去重後計數，狀態取回覆的 status；Read 以名為 Read 的工具呼叫計數，memory-keeper Read 以其父 Agent 的 subagent_type 為 chronos:memory-keeper 者計數；時長為日誌第一與最後時間戳之差；費用取 result 事件的 total_cost_usd。
時長｜第四輪：4.13 h｜第五輪：1.15 h（截至 2026-11-02T17:51:35Z，進行中）
費用｜第四輪：$10.16｜第五輪：未結算（日誌尚無 result 事件）
is_error｜第四輪：41（含 5 次取消）｜第五輪：4（截至同一時間）
review_code｜第四輪：27（completed 14、too_large 5、timeout 3、cancelled 5）｜第五輪：0
Read｜第四輪：84｜第五輪：30（截至同一時間）
memory-keeper Read｜第四輪：59｜第五輪：0（memory-keeper 尚未執行）
