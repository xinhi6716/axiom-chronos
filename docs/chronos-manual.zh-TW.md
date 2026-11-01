# Chronos Codex 使用說明書

截至 2026-11-01（台北時間）：已安裝 **0.2.2-codex.dev.11**，功能修復提交 `2929bd2`，事件日誌修正提交 `cc5269d228e8214db3c47067ca03825a3ae13fbb`，均已推送 `codex-only`；[Windows／Ubuntu 完整 CI](https://github.com/xinhi6716/axiom-chronos/actions/runs/41257830163) 均通過。版本化執行檔與安裝執行檔 SHA256 相同，重新啟動的 bridge 已回報 dev.11；舊對話的既有程序不代表熱更新。安裝收據位於 `chronos-artifacts/maintenance/dev11-contract-recovery/installation-receipt.json`。

**產品迭代完成數仍為 0／至少 5。** 第一輪已完成研究、意圖審核與規劃，曾卡在工程前基線；能力衝突已修復，但原 session `c308b6183976af5288614e4ba80fc863` 仍為 blocked，未自動恢復。修復測試不計入產品輪次。

## dev.11 修復內容（驗證與安裝狀態以交付收據為準）

- 新 session 的工程資料使用 `orchestrate/<project>-<session>/`；同 session 恢復沿用自身資料，不以最近更新的其他 goal 推測歸屬。舊共享目錄不自動搬移。
- 規劃者提交 `preflight.json`：`full_test` 必須與 modules.md 完全一致，列出需要的能力及凍結探測請求。這是契約相容性檢查，不代表測試命令已成功執行。每次預檢保存成功或失敗收據。
- 調度者可用 `workflow replan` 修正尚未開始工程的不可行契約；必須有短原因，保留核准研究與舊規劃目錄。已開始、未知結果或已有工程證據不得自動清除重來。
- `workflow installed_probe` 由 Host 啟動獨立 Astra HIGH 唯讀驗證。凍結請求使用 `role: astra-verify`、`effort: high`、`sandbox: read-only`、`task_kind: verification`。保存實際執行檔、雜湊、版本、程序與模型報告；相同輸入／來源／執行檔重放既有結果，未知結果不重派。這不等同另開 MCP 連線，也不自動判定答案正確。
- `submit` 只封存本輪工程驗收。核心依使用者授權完成 Git／安裝交付後，可以透過調度者交給 content 更新明確指定的專案 Markdown；不能趁機更動程式或驗收證據。最後才標記 goal done。

修復驗證（2026-11-01）：Windows／Linux 完整回歸與後續權限、session、交付門檻回歸通過，Rust 套件封裝驗證通過。三個新 session 的真實模型短測保留於 `chronos-artifacts/maintenance/dev11-contract-recovery/live-task*`：第一次暴露 Windows 深路徑寫入失敗；第二次暴露文件未寫入卻完成；第三次 `ed42adda0caa8a866953d1089c161726` 已跑通工程前 probe、工程／驗收、核心提交、content 寫入 RELEASE.md 與 goal done。最後完成要求實際 Host Write 見證及非空文件，不能拿先前工程成功代替文件交付。這是已曝光的開發回歸，並非獨立驗收或性能提升證明；新版安裝後仍須確認執行身分。

RSI 尚未完成可靠的自動演化閉環；上述修復不代表五輪完成，也不代表未量測的性能或 token 改善。修復期間允許直接維護插件來源，沿用分工、證據、低耦合與最小上下文原則；不要要求故障插件調度自己的修復。

## 暫存與編譯產物

此電腦的 Chronos bridge 與專案 shell 環境已設定 `TEMP`／`TMP` 為 `E:/codex/chronos-runtime/tmp`，`AXIOM_CARGO_TARGET_ROOT` 為 `E:/codex/chronos-runtime/cargo-tmp`。明確設定的 Cargo 暫存根無效時會報錯，不會悄悄改用 C 槽。短根目錄限制仍為 50 個 UTF-16 單位，以容納 Cargo 深層路徑。

dev.11 在可確認完成的驗證、成功封裝及完成的寫入 worker 後清理編譯產物，保留小型 `.chronos-binding.json` 作來源記錄；正式套件與測試收據在 session store 保留。未知、被中斷或尚未成功封裝的工作保留暫存供診斷，不宣稱所有失敗都已自動清理。

2026-11-01 清點的 C 槽 Chronos 相關暫存為 48.104 GiB；已可還原地移出約 48.104 GiB 至 `E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/backups/c-orphan-cache-20261101`，C 槽根目錄與使用者 TEMP 的相關目錄數已確認為 0。這是邏輯檔案容量，並非測得的實體磁碟回收量。搬移清單與收據在 `chronos-artifacts/maintenance/dev11-contract-recovery/c-*.json`；未刪除原始資料或正式安裝。

## 給新 task 的核心

1. 開啟 `E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex`，閱讀本說明書，再依使用者當次指令決定測試或迭代範圍。
2. 使用 Chronos wish／research 的原生 bridge 入口。目前對話就是核心，不另外建立核心或用原生 collaboration 取代調度。
3. 新任務建立自己的 session；只有使用者要求續接時才恢復舊 session。沿用已設定資料目錄，不搬動資料庫、不手改狀態或解除鎖。
4. 核心只傳達意圖、做最簡意圖複核與重大判斷，並提交通過驗收的輪次。工程與量測由調度者安排；發出指令後等待同一呼叫返回。
5. 「工程前獨立基線」使用下文 dev.10 量測入口，不呼叫必須等待工程完成的最終 tester。完成一次工具呼叫不代表通過測試。

閱讀本文件不構成啟動五輪迭代的授權。

## 核心功能與分工

Chronos 是 Rust 實作的 Codex 子代理調度與驗收系統。**目前這個對話就是核心；bridge 不另開核心。**

| 角色 | 職責 |
|---|---|
| 核心（Astra medium 即可） | 與使用者溝通、傳達意圖；只判斷研究是否違反意圖；確認重大問題／極佳方案；提交已驗證輪次 |
| Terra HIGH 調度者 | 路由、分派、依賴、持久化工作與一般修復；避免讓工程細節回流核心 |
| Astra MAX 研究者 | 完整研究、找錯與提出優化；獨立上下文，意圖通過後釋放研究執行緒訂閱與 Host 索引，保留研究產物 |
| Astra HIGH 規劃者 | 切分模塊、範圍、依賴與驗收條件 |
| 多個 Terra HIGH | 模塊實作；可獨立的工作並行 |
| Sol HIGH | 組裝與整合 |
| 另一個 Astra HIGH 驗證者 | 只驗證及授權量測，不設計改進、不修改工程 |
| Terra medium／Luna | 前者維護本地記憶；後者處理文件、圖片與大量非工程整理，不製作功能 |

正常流程：核心傳達意圖 → 調度者交研究 → 核心最簡 pass／reject → 規劃 → 實作 → 組裝 → 獨立驗證 → 核心提交。

- **Rust 狀態機與權限：**研究、意圖複核、規劃、工程、待提交、已提交；核心不能以角色名稱偽裝成工程代理。複核綁定研究 revision；`pass` 不附 reason，`reject` 附最短衝突原因。
- **持久化與恢復：**TODO、檢查點、操作所有權、階段收據與結果保留在磁碟。等待同一指令完成，不用原生 GOAL 自動續行、不輪詢模型進度。
- **可信驗收：**來源、契約、測試、獨立審查與 Rust 套件證據相互綁定；模型說完成不等於工程通過。一般修復留在調度子樹。
- **省 token／重用：**按角色提供精簡 brief、工具與技能設定；不相關插件、MCP、apps、skills 在子執行緒停用，並不移除使用者的官方整合。核心只接收必要摘要與權限邊界。已驗證且來源／契約仍有效的結果可重用。
- **本地記憶：**保存已驗證工作的摘要、接口、來源雜湊與專案歷史；先搜尋摘要、需要時才讀正文，能標示過期內容。不要宣稱是已驗證的向量 RAG 或完整自我進化核心。

## v0 到現在改了什麼

此處「v0」指最初 Codex 移植基線 `f3a846a`（套件 `0.2.0-codex.1`），不是正式的 `v0` Git tag。

| 階段 | 主要變更 |
|---|---|
| v0：初次移植 | 已有 Rust runtime、wish／research、bridge、工作單元與記憶；但仍會另啟核心，權限、停止與驗收證據邊界有缺陷 |
| 初期稽核修正（`8f7b513` 起） | 修意圖繞過、來源／審查不一致、dirty 範圍外修改漏判、並行歸屬、依賴與重用驗收；收緊子代理上下文與工具隔離 |
| dev.1–3：角色流程落地 | 改為目前對話持有核心；Rust 約束研究→複核→規劃→工程→驗證。修 ephemeral 研究執行緒無法 archive，改用 unsubscribe |
| dev.4–7：等待與驗收 | 加入跨連線 caller-command 鎖、有界調度續行與一般測試修復；修重複成功驗收使審查失效、輸入錯誤過早鎖住階段、重驗／重新封裝契約；完成新 task 實測 |
| dev.8：跨平台穩定性 | 修 Unix fork 繼承檔案鎖與取消／關閉競態；補 CI 依賴與實際啟動事件同步；倉庫整理為僅 `main`、`codex-only` |
| dev.9：斷線恢復與建置隔離 | 新增持久化 `session-call`，相同請求重播不重新呼叫模型；在取得獨占鎖且確認原 owner 已死時恢復研究／規劃；隔離工程子代理、Host 驗證、封裝及最終驗收的 Cargo 輸出，避免 `target/` 污染來源或觸發越界 |
| dev.10：解除基線死鎖 | 新增 Astra HIGH 隔離副本量測與來源／產物收據，允許在工程釋放前執行；保留最終驗收門檻。補正 Unix 夾具解鎖與 Windows 路徑比較，兩平台 CI 通過 |

詳細歷史與失敗證據見 [稽核紀錄](audit-2026-10-31.md)、[角色流程修復紀錄](role-authority-repair.md)。舊文中的「輪次」與已刪除分支名稱是歷史紀錄，不是本次五輪進度。

## 使用入口與迭代流程

1. 專案：`E:/codex/OPEN_DESIGN/axiom/axiom-chronos-codex`，分支 `codex-only`。只保留 `main` 與 `codex-only`。
2. 使用已安裝 Chronos 的 **wish／research 技能**，按技能呼叫 axiom bridge；不要用原生 collaboration 子代理取代調度。
3. 優先用原生 `session`。缺席時，按 wish 技能以 `axiom wish` 建立 session，再用 `axiom session-call REQUEST.json`，由持續執行的程序工具承接。**不要用 node_repl 承接長 bridge 呼叫**；它曾在外層 300 秒逾時後遺失連線。CLI 建立 session 成功不等於任務完成。
4. 每輪完整研究並記錄問題、證據、採納／捨棄原因、修改、同條件前後量測、代價、失敗與限制。未量測不得宣稱改善。保持低耦合、高內聚，不為湊輪次新增無關功能。
5. 每輪測試、提交推送、重建安裝並確認新版實際生效後，才開始下一輪。`workflow submit` 只是驗收狀態轉移，**不會自行完成 Git 提交／推送／安裝**。
6. 至少五輪；第六輪起，仍有至少三個有效問題／優化點，或核心確認一個嚴重問題／極佳方案，就繼續。這條跨版本迭代流程仍須實測，不可假設插件已全部自動完成。

## 現況與必要限制

- dev.9 本地 Windows、Linux 完整測試及 Cargo 套件驗證通過；310 秒等待後的程序中斷、恢復、結果重播測試通過，重播新增模型呼叫為 0。
- 第三個全新 task 在實際研究程序中斷後自行恢復，完整走到 `submitted`／`done`；前兩次失敗亦保留。這是同一已曝光小任務的驗收，不能推算一般任務成功率。
- dev.10 完整 CI 與 Rust 套件驗證通過；安裝後全新 bridge 的實際 Astra HIGH 量測完成兩個 Rust 測試，退出碼 0，原專案未變。這不等於原契約四個獨立基線案例已完成。
- 結果不明的工程操作仍保守阻擋；舊檢查點沒有 owner 證據時不自動重啟。Rust 鎖不能保證桌面宿主完全不重入模型或發等待訊息，也不限制核心在 Chronos 以外的原生工具。
- 正式 bridge 執行檔：`E:/codex/chronos-runtime/0.2.2-codex.dev.10/axiom.exe`；實際資料目錄沿用 `.codex/config.toml`，目前位於 `experiments/chronos-role-flow-dev7/home`，名稱舊不代表執行檔仍是舊版。
- 安裝與驗收收據：`E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/maintenance/dev10-baseline-fix/installation-receipt.json`。不要直接修改插件快取；改來源後重建。保留官方整合與原圖標。

這份說明書不會啟動工作；目前五輪迭代仍停止，須依使用者的新指示開始。

## 日常操作與異常處理

在目標專案的 Codex 對話中選擇 Chronos，描述「要改什麼、範圍、如何驗收、何時停止」即可。工程使用 wish，唯讀研究使用 research；由入口技能選擇可用傳輸，不需要使用者手動調度每個角色。

例如：「使用 Chronos，修正這個專案的 Unicode 字數計算，保留既有介面，測試通過後回報；不要推送。」這只是範例，不會因閱讀說明書而執行。

| 情況 | 處理方式 |
|---|---|
| 正在等待調度者 | 等同一指令返回；不開另一個 session、不啟動 GOAL、不定時查狀態或催促 |
| `intent_review` | 核心只對使用者意圖給 pass 或附最短原因的 reject |
| `ready_submit` | 核心檢查驗收摘要與授權，依流程提交；Git 推送與安裝仍依使用者授權另行完成 |
| 連線斷開、結果未收到 | 優先恢復原 session；CLI 路徑重送相同 request 檔與 ID，不修改內容、不另派同一工程 |
| `recovery_required` 或結果不明 | 保存 session、request 與錯誤證據並回報；不得清除鎖、手改階段或宣稱完成 |
| 取消或停止 | 遵守停止指示並保存既有檢查點；不得將 blocked 當 done |

## 檔案、狀態與更新

Pstack／Chronos 的狀態機管理工作階段與持久化狀態，**不是整台電腦的檔案寫入沙箱**。外部腳本自行建立的目錄不會被它自動收納。

- 正常呼叫沿用設定的資料庫與 session；重試不另開資料庫。TODO、檢查點、日誌、驗收證據使用引擎回傳的位置，不另建平行狀態檔。
- 必要的外部請求檔、測試沙箱、日誌與備份集中在 `AXIOM_HOME/artifacts/<session-id>/`；使用者指定產物目錄時以指定位置為準。正式交付檔案仍放專案約定位置。
- 本機既有歷史產物已集中在 `E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/`；`tests` 放測試、`evidence` 放日誌與收據、`backups` 放保存備份。已移除 14 個過期 junction 入口；舊文件中的歷史路徑可依 `maintenance/relocation-plan.json` 查找現在位置。僅 `E:/codex/chronos-iteration-evidence` 因舊 session 引用而隱藏保留，並非第二份資料。
- 活躍資料目錄與 runtime 保留原位；重開後確認只有 dev.10 bridge 存活。未來更新仍須確認新連線版本，不中止其他既有工作來強制更新。
- 入口的收納規則屬技能約束；dev.10 新增的隔離量測與來源檢查則由 Rust 執行。
- 修改插件須改來源，再經正式安裝流程更新；不直接編輯快取。只有 Rust／內嵌引擎有改動才需重新編譯執行檔；每次更新需核對來源、套件與實際連線版本。

## dev.10：工程前基線量測

修復「工程前必須測量，但驗證角色禁止跑測試、最終 tester 又要求工程完成」的循環依賴。

調度者在 planning／execution 階段使用 `codex_run`：`role: astra-verify`、`task_kind: measurement`、`sandbox: read-only`、原專案 `cwd`，並在 brief 指定測試與量測要求。這裡 read-only 表示不得修改原專案；Host 實際建立獨立可寫副本，讓 Astra HIGH 執行測試與保存新增的量測檔案。它不得修改既有程式或提出工程改進。

Host 封存包含未提交變更的 Git 可見來源，保存原始檔案與量測產物 SHA256、模型回報及前後來源指紋。原專案改變或副本原始檔案被改寫時回報失敗。收據存於資料目錄 `artifacts/<session-id>/measurements/<id>/`（無 session 的直接 bridge 呼叫則在 `artifacts/measurements/<id>/`），不要求先釋放工程單元，也不授予工程完成或最終驗收資格。

限制：不包含 Git 忽略的輸入；符號連結與 submodule 拒絕處理；data home 必須在原專案之外。測試結果仍需查驗實際命令、退出碼與原始輸出，模型 turn 完成不等於測試通過。量測入口可解除本次基線能力衝突，但不提供任意階段回退，也不重啟結果不明的工程。

本次驗證涵蓋待施工單元仍未釋放時執行量測、dirty 來源複製、越權修改副本失敗、核心與審核前角色拒絕，以及實際 Astra HIGH 執行兩個 Rust 單元測試。這些是修復驗證，不是五輪產品迭代或原契約四個獨立基線案例的完成紀錄。

## 本機資料夾索引

| 位置（相對於 `E:/codex/OPEN_DESIGN/axiom`） | 用途 |
|---|---|
| `axiom-chronos-codex/` | Codex 專用版來源，分支 `codex-only` |
| `axiom-chronos/` | 原版來源，分支 `main` |
| `experiments/chronos-role-flow-dev7/home/` | **使用中的持久化資料庫，名稱保留，不移動** |
| `chronos-artifacts/tests/` | 已封存的測試專案；不用已移除的舊入口續跑 |
| `chronos-artifacts/evidence/`、`backups/` | 歷史量測、日誌、收據與備份 |
| `chronos-artifacts/maintenance/dev10-baseline-fix/` | dev.10 修復、失敗案例、安裝與 CI 證據 |
| `chronos-artifacts/tests/chronos-five-rounds/active-runtime.json` | 本機 runtime 版本與收據指標，不是自動續行開關 |

`axiom-forge/`、`outputs/`、`work/`、其他 `experiments/` 內容與上層其他專案未搬動。歷史測試的信任路徑已改指向封存位置；既有日誌與來源綁定證據不改寫。原始碼、官方整合、活躍資料庫及執行檔未因這次整理改動。
