# Chronos Codex 使用說明

版本：`0.3.0-codex.dev.7`，從標準版 `e27de7a` 重建的開發候選。

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
