# Codex 重建參考

以下保留已核准研究的參考資料與採納邊界。U1–U4 已經 SOL 串接成實際 Rust 動態 RPC pipeline；本地回歸不等於安裝後的新聊天獨立驗收，未重審外部倉庫。

## 本地基礎

標準版 e27de7a5e971e5e9317bb338088e79ad9f9bef6e：實讀 src/codex/rpc.rs、run.rs；src/bridge/server.rs、tools.rs、codex_tool.rs；src/session/run.rs、render.rs；src/orch/store.rs、lifecycle.rs、evidence.rs；src/memory/mod.rs，以及 base、engine、CLI、CI 與相關測試入口。依據包括 Claude 啟動／GOAL 續跑、拒絕反向 RPC、記憶體 RUNNING claims、同 family review 阻擋、SOL failing-full 前置條件與 rewrite done gate。[標準版 RPC](E:/codex/OPEN_DESIGN/axiom/axiom-chronos/src/codex/rpc.rs)

舊版 75e344b7987707df7d514842d3a2fbff75376a17：實讀 src/codex/rpc.rs 的 ToolHandler、callback 路由、完成屏障及巢狀呼叫測試，另讀 run.rs 的部分政策／取消處理。採納範圍只限 RPC 小段機制。

外部資料位於 [references](E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references)。下列 ref 為隨附 tree.json 記錄的快照參照；實讀是列出的範圍，不代表全庫審查。

### 1. deepseek-ai/dsh-superagent-prototype

ref：6337fc823ce20251c8de8085948165c81aaa701e，另由 commit patch 確認。

實讀：README；docs/003-execution.md 的階段、等待、重試／重啟與 stored-work 段；src/app/sessions.ts 的 ensure、工具限制、skills 綁定與 release；固定 commit 的 src/core/assignments.ts；線上 src/core/engine.ts 的 atomic 入口。

採納：先記錄工作再產生效果、assignment／輸入身分、角色工具集合、執行資源釋放，掛靠 U1 callback 與 U2 checkpoint。捨棄：danger-full-access、主機重啟直接建立 replacement assignment、外部訊息／HTTP／UI 堆疊；未知寫入不能靠恢復提示詞解決。[assignments.ts](https://raw.githubusercontent.com/deepseek-ai/dsh-superagent-prototype/6337fc823ce20251c8de8085948165c81aaa701e/src/core/assignments.ts)

### 2. stablyai/orca

commit：b10a6fe8275b2760ab9e9f52c018721f7d88ab88（後續確認）；沒有全量 clone。

實讀：README 功能範圍；本地 src/main/agent-turn-idle-verdict.ts、agent-awake-status-lease.ts、agent-state-file-reader.ts 全部實質程式。三檔本地 SHA-256 分別為 f72e0377e8733b4cea0e29db05755db8f8f1eac564defb455efccc5372a57217、871bab7b945f84612b2b170a912371756b6079b13c7e7940f05f4d4ed706984f、788445c173ef74320639acd9adc1c5a1ae934c5976e01146c237c6e207516326。

採納：舊 session 狀態不能充當本輪完成、新 runtime 要區分新觀察、狀態檔讀取有界，掛靠 U1 事件身分與 U2 恢復。捨棄：無狀態終端經 grace period 就視為 completed 的 heuristic；Chronos 必須有對應 terminal 證據。不引入 Electron、PTY/UI、daemon 或 workspace 平台。[本地實讀源碼](E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/references/stablyai--orca/src/main/agent-turn-idle-verdict.ts)

### 3. DeusData/codebase-memory-mcp

ref：f29e74f276af08456ba80c3b8e21f466bd00765b。

實讀：README；docs/MEASURING_SAVINGS.md 的評估分類與凍結實驗段；src/daemon/project_lock.c 的 key／acquire／release；src/pipeline/pipeline_incremental.c 的 incremental 概述、semantic manifest 與穩定內容雜湊段，未通讀整個 pipeline。

採納：內容與來源快照綁定、寫入互斥，以及品質／系統延遲／agent 成本分開量測；掛靠既有 memory hash、orch scope／evidence。延後：完整程式圖、Tree-sitter、多語言索引、daemon；標準版記憶功能尚無量測證據需要它們。[增量源碼補讀](https://raw.githubusercontent.com/DeusData/codebase-memory-mcp/main/src/pipeline/pipeline_incremental.c)

### 4. msitarzewski/agency-agents

ref：b37b5bb33b78f2c1d6567d98e25ca818e3259219，另核對 commit 頁。

實讀：README；specialized/agents-orchestrator.md 前 85 行；testing/testing-reality-checker.md 前 100 行；線上 integrations/codex/README.md 與 scripts/convert.sh 的 convert_codex、來源枚舉／slug 碰撞處理。

採納：小型角色交接成果與以實證核對報告，落在 engine brief 模板與 U3 profile。捨棄：整套人格、必須反覆返工、固定評級、網頁 screenshot 規則與完整背景傳遞。convert_codex 實際只轉成 name／description／developer_instructions，並沒有可靠調度或權限 runtime，不能拿來替代本案 Rust 接線。[轉換實作](https://raw.githubusercontent.com/msitarzewski/agency-agents/main/scripts/convert.sh)

### 5. trailhq/Trail-Code

ref：a44da3dfb5894c8e316e320526041ef8e5ddbb8c。

實讀：README；src/context/check.ts；build.ts 的內容快取、manifest 與原子保存段；savings.ts 的 baseline／toTokens／缺失價格處理；線上 src/graph/extract-cache.ts 的版本及 extractor-stamp 設計。

採納：新鮮度必須綁內容與產生器身分、索引／摘要是可失效資料、完整內容按需讀；直接掛靠 memory stale 與 brief artifact。延後圖索引與摘要快取擴張。捨棄把 chars/4 或假設整檔閱讀量當實際 token 節省；本案只報量測 usage。[extract-cache.ts](https://raw.githubusercontent.com/trailhq/Trail-Code/main/src/graph/extract-cache.ts)

### 6. lennney/stop-that-shit

ref：e4fe5296f6917849f7eec0860d6a1b68bc0292de。

實讀：README；ARCHITECTURE.md 的 control／state／delegation 邊界；HOST-ADAPTER-CONTRACT.md 前 65 行；src/contracts.cjs 的預設、指令解析與 normalize；線上 src/delegation-state.cjs 的 reservation／action 去重與子身分綁定。

採納：要求與事實分離、未知完成保留占用、重複事件不能釋放後來的 reservation，落在 U2 Attempt 與 scope 保留。捨棄：通用自然語言授權解析、七宿主 adapter、完整 contract／guard 系統；Chronos 只需固定 purpose 與受控工具入口。[delegation-state.cjs](https://raw.githubusercontent.com/lennney/stop-that-shit/main/src/delegation-state.cjs)

### 7. affaan-m/ECC

ref：b38be6a7f6350964d59328b52f0624e1d1d552d1，另核對 commit 頁。

實讀：README；docs/token-optimization.md 前段；docs/design/context-carriers.md 的選取、資源閉包與結構／原生驗證邊界；線上 scripts/hooks/pre-compact.js 的 exact-worktree 選取、輸入上限及摘要保存路徑。線上 main 源碼與本地固定文件不混稱同版本。

採納：按需選技能且附屬資源要完整；記憶／checkpoint 綁 canonical repo；檔案結構通過不等於插件真的載入，掛靠 U4 安裝 smoke。捨棄：全域模型／hook 設定、自動 observer 學習程序、壓縮時再叫 LLM 生成一份權威狀態。Rust checkpoint 已能保存精確進度。[pre-compact.js](https://raw.githubusercontent.com/affaan-m/ECC/main/scripts/hooks/pre-compact.js)

整合落點：`src/bridge/dispatch.rs` 的固定 purpose 角色、動態 callback 與階段接線；`src/codex/{rpc,transport}.rs` 的身分／終態屏障與 scratch 程序 cwd；`src/orch/checkpoint.rs` 的計畫前唯讀 claim、凍結快照、恢復與驗證門檻；既有 memory 與 Pstack 的真實證據投影。未引入新依賴、第二核心、原生 collaboration 或通用 receipt DSL。實作採同供應商的獨立 role／attempt／thread review，不再要求跨 family 或故意失敗的 fulltest。
