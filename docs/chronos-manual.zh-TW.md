# Chronos Codex 使用說明

版本：`0.3.0-codex.dev.10`，從標準版 `e27de7a` 重建的開發候選。

## 核心與分工

目前的 Codex 對話就是 ASTRA medium 核心：傳達意圖、對研究給出通過／不通過及最短理由、作重大判斷，並在驗收後提交。工程與主要設計不回流核心。

| 工作 | 角色 |
|---|---|
| 調度 | Terra medium，唯讀，使用指定派工工具 |
| 研究 | 獨立 ASTRA max |
| 規劃與拆分 | 新的 ASTRA high |
| 工程 | Terra，依不重疊範圍並行 |
| 組裝 | SOL high |
| 驗證與指定量測 | 另一個 ASTRA high，不修改產品 |
| 本地記憶 | Terra medium，只保存已驗證成果 |
| 非工程文件、圖片及批次內容 | Luna，不製作功能或工程程式 |

Rust 保存階段、工作單、範圍占用、嘗試身分與證據。子代理取得精簡 brief、必要角色資源與相關記憶摘要，不帶核心聊天歷史。Lean 要求停用無關插件與技能目錄，不等同隔離宿主提供的所有原生工具。

## 呼叫與完成

在專案使用 `@Axiom Chronos` 描述要完成的事。CLI 保留 `axiom wish <意圖>`、`axiom research <問題>`；`--project PATH` 放在意圖之前。`axiom doctor` 檢查入口，不代表完整工作流已驗收。

wish 在研究完成後返回 `needs_intent_review`。核心核對是否符合意圖，以研究 digest 明確決定。否決會把保留的草稿及理由送回研究，停在新的複核邊界；通過才進入規劃與工程。research 到研究邊界即交付，不自動啟動工程。

規劃先固定驗證命令與範圍，基線在工程前可合法執行。組裝後，由新驗證者要求 Rust 執行固定 check ID，驗證程序只可寫隔離暫存目錄。已知驗證失敗可返回 SOL，最多重修兩次，每次重新獨立驗證；不更換驗收命令，不重派未知寫入。

通過並保存記憶後返回 `ready_to_commit`，由核心提交。一般專案不必有遠端或 Rust 發版流程。一個 run 只接收目前一輪的交付要求；其餘輪次留在簡短 TODO。若任務要求多輪，核心完成該輪指定的提交、推送、套件更新與版本確認後，再提出下一輪意圖。本次重建不算先前要求的五輪迭代。

## 等待與恢復

同一工具呼叫等待實際事件，不用 GOAL 自動續行，不反覆要求 LLM 查進度；執行本身仍有時間預算。`status` 只讀 checkpoint，不啟動模型。

斷線後保留 request ID、run ID 與原輸入，可重送相同 request，或用 `axiom wish --resume <run_id>`。未知結果須查核保存的報告／終態；`--recovery-file` 提供綁定身分的停止、修復證據。不可直接改 checkpoint 或清掉占用；blocked 也不允許改走舊 codex_run／orch 寫入。Bridge 依專案及實際倉庫路徑拒絕這類接管，但無法禁止核心繞出插件自行使用宿主 shell。程序死亡時不保證自行喚醒核心；持久化是恢復紀錄，不是作業系統服務。

`--status <run_id>` 查詢；`--cancel <run_id> --reason <理由>` 取消。意圖審核使用 `--decide <run_id> --research-digest <digest> --verdict pass|fail --reason <理由>`。

`--creative`、`--astra`、`--no-architect` 保留為意圖提示，不會繞過指定角色流程。`--max-turns` 明確拒絕，此版不另啟可限制回合數的核心。`gemini_preview` 與 `review_code` 名稱保留為 Codex Astra 相容入口。

## 資料與安裝

`AXIOM_HOME` 指定根目錄；`AXIOM_DATA_HOME` 指定工作資料目錄，未設定時使用前者的 `data` 子目錄。資料含 `orchestrate/` 的工作單、checkpoint、證據與暫存結果，以及 `memory/` 的 SQLite 記憶庫。設定集中目錄，避免散落專案及系統磁碟；保留舊資料，不以新格式覆寫舊 session。

從來源執行 `cargo build --release --locked`。`engine/` 是角色與技能來源，`codex-plugin/` 是發布包，兩者一起更新；用 `codex plugin add chronos@axiom-chronos-codex --json` 安裝，不改快取。Rust 執行檔、插件版本與 MCP 設定需一起更新；執行檔放在版本目錄，避免覆蓋正在使用的檔案。保留官方 Codex 插件與服務。

檔案或版本字串不代表新版已載入。發版前確認 Windows／Linux CI、執行檔身分、實際 MCP 入口與新 chat 工作流。假模型測試不算獨立驗收，未量測不得宣稱省 token 或加速。

## 從原版到本版

- 標準版：Claude 核心／Sonnet 調度、Codex 工程、Haiku 記憶、可選 Gemini 整理報告；已有 Rust Pstack 與記憶庫。
- 舊 Codex `0.2.2-codex.dev.*`：多次補上權限、等待、恢復與交付契約，出現循環門檻及恢復死鎖。歷史保留在 Git 與測試證據，失敗輪次沒有記成成功。
- 本版：重新以標準版為底，替換宿主與角色接線，集中使用一套 checkpoint。加入綁定身分的動態 RPC、實際終態屏障、凍結命令的隔離驗證、研究修訂與有界返工、可失效記憶及恢復。保留原圖標，未新增外部框架或依賴。

七庫的實讀範圍、採納／捨棄理由與來源見[重建紀錄](codex-rebuild.zh-TW.md)。

開發候選 dev.1 的新 chat 測試曾將三輪放進同一 run，造成提交邊界衝突並繞行舊工具；產品三次提交不算工作流通過。dev.2 修正逐輪入口、明確 research 欄位及舊寫入入口保護；獨立重測結果另行記錄，不預先宣告成功。

dev.2 的新 task 正確維持逐輪／權限邊界，但暴露驗證器覆寫凍結 CARGO_TARGET_DIR，建置與啟動路徑不一致；因此工作流仍未通過。dev.3 保留凍結環境，只補未指定的暫存預設，並加入自訂建置位置的回歸檢查。

dev.3 的三項凍結檢查通過，但工程測試單元在尋找檢查定義時讀到相鄰驗收資料，獨立性失效，未列成功。dev.4 將核准計畫的懶載入參照提供給工程／組裝單元；新測試的驗收資料另放工作區外，舊案例保留為已曝光回歸資料。此為參照與工作區安排，不冒稱作業系統讀取隔離。

dev.4 的實際 PowerShell 驗證暴露 Windows verbatim 根路徑混用斜線，導致 Select-String 讀取失敗。dev.5 對命令佔位符使用可攜顯示路徑，canonical 路徑仍保留作身分比較，並加入真正的 PowerShell 路徑回歸測試。

dev.5 的工程單元依規劃中的相對 scratch 路徑寫出範圍而被拒絕。dev.6 改由 Rust 為每個寫入 attempt 分配倉庫外暫存根、注入建置／暫存環境並授予該根寫入；產品 scope 不擴大，違反時保留明確 scope_violation。設定依據：[OpenAI 設定參考](https://learn.chatgpt.com/docs/config-file/config-reference)。

同版移除 Windows 的 PID-only taskkill /T 清理，改以父子建立時間、快照時間上限與持有的 handle 驗證後終止子程序。dev.5 CI 曾異常退出，兩者因果尚未證實；不得把該次 CI 寫成成功。實際 Terra 暫存環境短測通過，產品目錄保持空白；證據保存於重建資料目錄。

dev.6 通過 179 項本地測試及 Windows／Ubuntu CI，但新 task 僅完成第一輪；第二輪將指定 scratch 內的測試腳本誤判為產品越界，尚未完成三輪。dev.7 對寫入回報允許該 attempt 的絕對 scratch 路徑，解析連結與已刪除檔案的存活父目錄，仍拒絕其他 attempt、相對路徑及上層跳脫。回歸模型現在也回報暫存檔案事件，避免僅測 shell 寫入而漏掉 apply_patch 回報。新 task 驗收結果另行記錄，不預先宣告通過。

### dev.7 重建驗收（2026-11-01）

- 產品 commit：`9db1414c40cea34866db521ddff13cd53ef40051`，版本 `0.3.0-codex.dev.7`。180 項本地測試、[該 commit 的 Windows／Ubuntu CI](https://github.com/xinhi6716/axiom-chronos/actions/runs/41298645363) 通過；來源套件、安裝快取與 Rust 執行檔身分已核對，新 MCP 程序回報相同版本。
- 全新 ASTRA xhigh task `01a196b6-e180-7152-84d6-08aa67b29818` 僅收到三輪 linecount 使用者任務，沒有舊 checkpoint 或中途補指令。三輪各自完成研究、意圖複核、規劃、工程、SOL 組裝、獨立驗證與記憶，皆達 `ready_to_commit`；提交依序為 `0b8d5cb`、`980aea6`、`e02868d`，最終工作樹乾淨。
- 另逐一重建三個提交，執行未提供給 worker 的 28 次 CLI 檢查，28 次通過。角色與 checkpoint 核對為 3／3 通過；核心呼叫只有三組 `start`／`decide`，沒有 GOAL、status 輪詢、舊工具接管或核心產品修改。
- 原始結果在本機 `E:/codex/chronos-runtime/acceptance/codex-rebuild-dev7/`：`frozen-scenario.json`、`fixture.json`、`holdout-results.json`、`workflow-results.json`、`chat-audit.json` 與逐提交建置輸出。安裝／CI 收據在 `chronos-artifacts/evidence/codex-rebuild-20261101/installation/dev7/`。
- 限制：只有一個 Windows 實際任務樣本，耗時 1,867.626 秒（約 31 分鐘）；不能據此宣稱效能或 token 改善。此測試證明三輪受管流程可完成，不證明程序崩潰後會自行喚醒，也不代表 Rust 能封鎖宿主核心的原生工具或免除宿主等待期間的模型續行。正式產品三輪迭代另以新 task 開始，本次重建與短任務驗收不計入。

### dev.8：獨立驗收準備與規劃恢復

正式迭代的第一輪在規劃階段停止，產品輪次仍為 0／3。原因是流程要求工程前封存獨立驗收，卻沒有建立該材料的步驟；既有 known-failed 規劃也無法合法重試。dev.7 短測由外部觀察者預先準備驗收案例，沒有覆蓋這項能力，不能拿該次成功證明插件能自行建立驗收。

dev.8 由規劃者以 `independent_evaluation` 提出需求，再由另一個唯讀 ASTRA HIGH 驗證者產生自含的驗收指令與新案例。Rust 在基線與工程前保存並固定完整命令及輸入雜湊；執行時在隔離 scratch 建立所需材料。公開計畫只包含回歸檢查與驗收方法，工程／組裝拿不到私有命令內容或驗收報告參照；組裝改讀明確的工程／內容成果參照，不遍歷全部角色報告。這是傳遞內容的隔離，不冒稱宿主檔案系統讀取權限的全面封鎖。若資料實際曝光，獨立性仍須宣告失效。

未固定計畫、尚無工程寫入、全部嘗試終止且研究已通過時，可用 `resume` 的 `recovery: {"retry_planning":{"reason":"具體修復或新事實"}}` 重試規劃，最多兩次。原研究、失敗報告與歷史結果保留；新規劃使用新 step／attempt 身分。這不允許修改已凍結的計畫，也不重派未知寫入。動態工具失敗會保留原始錯誤原因。

回歸測試涵蓋從零準備私有驗收、準備先於寫入、作者上下文排除私有案例、規劃恢復不重跑研究且保留失敗，以及拒絕越過已凍結計畫／未知寫入。另以全新 task 驗收實際能力，結果需另外記錄；這次修復不計為正式三輪之一。

### dev.9：業務成功判定與失敗路由

dev.8 的真實任務在準備階段失敗，卻被當成完成：準備者明說失敗並回傳固定 exit 1 的佔位檢查，Rust 只檢查 checks 非空，就安裝成成功準備；基線只確認收據齊全就放行工程；工程者回報「未開始工程」，回合正常結束仍記為 completed，依賴因此解鎖；驗證失敗又不分來源丟給 SOL 返工兩次。產品最後仍為空。

dev.9 把「回合終結」與「工作成功」分開，並讓失敗回到能處理它的邊界：

- 準備契約改成型別化 `{status, summary, exposure, checks}`。`failed` 直接返回具體前置失敗，不安裝任何檢查、不啟動工程；這仍屬規劃階段，可依 `retry_planning` 合法重試。`ready` 需有非空 exposure、每個檢查都實際讀取產品 `{repo}`，且不得是公開回歸命令的換名重複。
- 寫入角色（Terra 工程、SOL 組裝、Luna 內容）改回 `{status, summary, artifact}`。`blocked` 不滿足依賴、不進入組裝與驗證；`completed` 允許「原本已符合要求」而無需改檔。回合正常結束不再等於工作完成。
- 基線執行全部凍結檢查後，由驗證者對每個實際結果分類為 `passed`／`product_missing`／`product_defect`／`infrastructure_failure`，Rust 再與實際 exit code 交叉核對。設施失敗或無法判定即阻止工程；只有另存的 `gate-baseline` 收據才授權寫入。空產品的新功能仍可先記為 `product_missing` 再合法開工。
- 工程／組裝 brief 的設計是提供非洩漏的前置證明（檢查數量、私有檢查數量、設施已就緒），不附私有案例，也不再要求等待準備或基線報告。下方實測沒有保存完整執行期 brief，無法直接核實實際內容。
- 驗證結果新增 `defect_owner`。`assembly` 才回到 SOL 返工；`upstream` 或 `none` 直接 blocked 並保留凍結計畫與報告，不再消耗無效返工。
- 動態工具 callback 保留具體失敗訊息，首次 blocked 的 `short_reason` 可顯示業務原因。實測仍有型別被泛化、後續狀態查詢遺失原因的缺口，見下方紀錄。

限制：隔離仍是上下文層級，不是作業系統保證。型別與機器檢查無法證明私有案例在語意上一定有效；決心造假的準備者仍可能在 `ready` 下交出無實質覆蓋的檢查，只是固定 exit、佔位或抄公開檢查會被拒絕，最終仍由獨立驗證者與凍結檢查把關。dev.9 未處理長等待與斷線喚醒（dev.8 報告 F8），也沒有量測 token 或速度改善。正式三輪迭代仍未開始。

#### dev.9 真實驗收紀錄（2026-11-01）

- 原始 `sumints` run `run-3ec46c705c1083c78121a1e4` 在基線判定為 `infrastructure_failure` 並 blocked，沒有啟動工程。三項公開檢查因空倉庫而判為 `product_missing`；獨立檢查的控制樣本在 MSVC linker 發生 `LNK1104`，未進入產品驗證。[checkpoint](E:/codex/chronos-runtime/data/0.3/orchestrate/sumints-acceptance-dev9-20261101/runs/run-3ec46c705c1083c78121a1e4/checkpoint.json)、[基線報告](E:/codex/chronos-runtime/data/0.3/orchestrate/sumints-acceptance-dev9-20261101/runs/run-3ec46c705c1083c78121a1e4/reports/d8360f2dc4281fefc91271cc719cd632.json)。首次 blocked 訊息保留原因，但頂層錯誤型別被泛化為 `tool_failed`，後續 `status` 也沒有還原具體原因。
- [長短路徑合成對照](E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/diagnostic-windows-link-20261101/README.zh-TW.md)在同一工具鏈下重現 `linker-arguments` 的 `LNK1104`，支持長路徑是原因；原始程式與執行條件不同，不能排除其他因素。該對照已曝光，僅為診斷，不能當獨立驗收。
- 原宿主沿用相同 fixture 的替代 run `run-a0d884a985d4023530a0ead5` 因計畫中的 `h` 與私有檢查必須以 `independent-` 開頭的契約衝突而 blocked；重試收據雖保存修正理由，新的規劃 brief 未收到它，準備再次失敗。[準備報告](E:/codex/chronos-runtime/data/0.3/orchestrate/sumints9/runs/run-a0d884a985d4023530a0ead5/reports/5c18c058b7e8042eacb243c0e884091b.json)、[重試收據](E:/codex/chronos-runtime/data/0.3/orchestrate/sumints9/runs/run-a0d884a985d4023530a0ead5/planning-retry-1.json)。另一個 run `run-8b66dc00a4d3c279fc4f8464` 的獨立 runner 有 Rust 語法與型別錯誤，在基線被判為設施故障並 blocked。[基線報告](E:/codex/chronos-runtime/data/0.3/orchestrate/sumints9/runs/run-8b66dc00a4d3c279fc4f8464/reports/81ab52cbcd69fbc8793ad88c74565cb9.json)。這些替代 run 沿用 fixture，不能算全新未曝光驗收。
- 另一次 `run-438317c61bda6f5ca0c52291` 在基線啟動檢查程序時，Windows `CreateProcessAsUserW` 拒絕存取；基礎設施未就緒而 blocked，沒有啟動工程。[基線報告](E:/codex/chronos-runtime/data/0.3/orchestrate/sumints9/runs/run-438317c61bda6f5ca0c52291/reports/552d077e563c929ac066659434a2e91a.json)。
- 後續同一 fixture 的 `run-b83fab4591e0a2f2cca0409f` 到達 `ready_to_commit`，原宿主本地提交 `a821b8e726b5175f7d2efc033ee3e52211640bf4`。四份終驗收據的 exit 均為 0，驗證報告回報 `pass=true`、`defect_owner=none`；事後觀察者重新建置、執行回歸與六個 CLI 案例均通過。這是單一沿用 fixture 的已曝光診斷交付，整體新版驗收仍待新 fixture。[彙整與證據](E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/codex-rebuild-20261101/acceptance-evaluation-dev9/consolidated/result.json)。
- 準備報告回報 `status=ready` 並填有曝光說明；四項基線均回報 `product_missing`，此樣本未測到其他基線分類或負向返工路由。匯出紀錄稱 43 組案例中一組與已記錄開發輸入完全重疊，其餘 42 組未發現完整 argv 重疊；因缺少完整讀取稽核與實際送給 writer 的完整 brief，不能證明私有案例未曾暴露。
- 原實測為單機 Windows 的 headless `codex exec` task，核准使用 `--approve-for-me` 自動審查，不是 GUI task。`f2eef18` 的 [Windows／Ubuntu CI](https://github.com/xinhi6716/axiom-chronos/actions/runs/41310739823) 通過；先前 Windows 曾出現 timing flake。沒有可比較的 token 或速度量測，不能據此宣稱改善。正式三輪迭代尚未開始。

### dev.10：維護中（尚未發布）

以下描述工作樹中已實作的行為，不代表新版已安裝或通過驗收。

- 動態工具保留具體錯誤型別與原因，blocked／outcome unknown 的原因另存並綁定 checkpoint 版本；`status` 與相同 request 的重送會讀取可核對的紀錄。舊紀錄若沒有原因則回報未知，不補造原因；舊的成功結果重送時仍須符合目前的驗收門檻。
- 新 scratch 路徑以完整 run／attempt／check 身分雜湊縮短，舊 run 的既有 scratch 經身分及連結檢查後可沿用。縮短路徑不保證所有工具鏈的長路徑問題均已排除。
- 規劃重試依序帶入已核准的修復理由，不能藉此改變已凍結檢查。檢查環境變數依平台規則判斷重複名稱：Windows 不區分大小寫，其他平台區分；暫存預設不覆蓋明確指定的值。私有檢查保留 `independent-` 前綴，須包含 `{repo}`，且不能與公開檢查使用相同 program／args／env；這些形式檢查不能證明案例語意獨立或真的讀取產品。
- store 使用 OS 鎖與完整 PID 標記；只有確認程序已結束才自動取代舊標記。明確 `force` 仍可繞過持有中的鎖並與寫入重疊；舊版本不遵守新 OS 鎖，混版不能保證互斥。PID 標記以 hardlink 發布，檔案系統不支援時會失敗。
- `tree-v2` 對工作樹中的追蹤、未追蹤及被 Git 忽略檔案做內容雜湊，也納入目錄、空目錄與 Windows NTFS 命名資料串流；`.git` 中繼資料不在此範圍。無法安全讀取或遇到不支援的連結時停止，而不是忽略輸入。這需要完整磁碟讀取；使用者已接受讀取目前約 36.9 GB 的建置目錄，實際耗時與總讀取量仍待量測。這不是作業系統快照，也不涵蓋倉庫外的所有輸入。
- 舊 `tree:` 快照保留為歷史證據，但不能在新門檻下授權繼續寫入或驗收；未決嘗試須先核對，再用新 run 建立完整基線。基線、終驗及審查重新核對目前有效的來源、固定命令、角色身分與收據；工程／組裝結果須有原始寫入範圍稽核。只有當前獨立審查明確歸因 `assembly` 的失敗可返工，最多兩次，不能改驗收命令。
- 專案記憶保存版本化完整輸入 manifest 與驗收 source／review digest，重用時以相同語意檢查；先篩選相同 canonical repo 的有效記憶，再取最多三筆。來源未知或損壞的單筆記憶略過，沒有搜尋詞時不阻擋規劃。舊 unit 的檔案雜湊仍可判定，舊 run 不完整來源不自動重用；legacy 寫入也重算目前 review 與必要 acceptance。
- 模型收據記錄請求模型與伺服器回報的 thread 設定模型，兩者須一致並綁定派工身分；這不是每回合實際使用模型的遙測。
- 獨立驗收準備可選擇一次受管 Rust runner 編譯／連結，使用固定命令；準備時不執行 runner、產品或測試案例，也不是任意命令工具。編譯失敗時不自動改用其他檢查；編譯通過只證明該次編譯與連結，Rust 巨集仍可能讀取宿主檔案，不能視為 OS 讀取隔離或驗收通過。
- 真實 Windows／Codex 0.159.3 協定診斷已驗證固定命令可在 scratch sandbox 編譯、連結及執行同 SHA-256 的 runner，指定的 scratch 外寫入遭拒；設定為 `service_tier=default`，零模型 thread／turn。這是受控能力診斷，未測網路阻斷或 OS 讀取隔離，也不代替完整工作流驗收。[原始證據](E:/codex/OPEN_DESIGN/axiom/chronos-artifacts/evidence/runner-os-sandbox-protocol-20261101/run-02/summary.json)。

終態入口共同驗證 turn 與模型收據；未派送的 `Prepared` 可保存取消證據。設定修復後須明確 Resume，且只對可核對實際 callback、coordinator attempt 及整批成員的失敗唯讀調度者開放重試；未知 writer 仍禁止重派。模型與事件證據採內容定址，孤立檔案不覆寫已引用證據，也不阻擋新的 thread。這些恢復路徑已通過定向回歸；完整測試套件、Windows／Linux CI、安裝與新 fixture 的未曝光驗收均待驗證，目前沒有可比較的效能或 token 量測。

