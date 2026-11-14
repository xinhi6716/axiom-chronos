<p align="center"><img src="https://raw.githubusercontent.com/xinhi6716/xinhi6716/95b148368534587e5393bc9ef610b99c01ca8e92/assets/identity-v8/chronos.png" width="280" alt="Axiom Chronos: an hourglass beneath a celestial timewheel"></p>

<h1 align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/xinhi6716/xinhi6716/443d48c713f27d52656ad8ea3cb4c5bb9c7dd440/assets/brand-marks/v1/chronos-icon-dark.svg"><img src="https://raw.githubusercontent.com/xinhi6716/xinhi6716/443d48c713f27d52656ad8ea3cb4c5bb9c7dd440/assets/brand-marks/v1/chronos-icon-light.svg" width="40" height="40" alt=""></picture> Axiom Chronos</h1>

<p align="center"><strong>讓 Claude 與 Codex 一起規劃、實作與審查程式修改。</strong></p>

<p align="center"><a href="README.md">English</a> · <strong>繁體中文</strong></p>

<p align="center"><a href="docs/releases/3.0.0-beta.1.md">預先發行版 3.0.0-beta.1</a> · <a href="docs/releases/2.0.1.md">最新穩定版 2.0.1</a></p>

<p align="center"><a href="#begin">開始</a> · <a href="#desktop-app">桌面應用程式</a> · <a href="#the-work">工作流程</a> · <a href="#record-review">紀錄</a> · <a href="#status">狀態</a></p>

對你的程式碼許一個願望。Chronos 會先規劃、切成模組、交給 Codex 施工、審查每個單元，一直做到修改驗證完成或確定卡住為止。

程式碼索引為原生實作，不需要外部服務。由於 tree-sitter 在離線環境無法取得，Rust 改用 syn，Python、TypeScript 與 JavaScript 則使用手寫詞法分析器。詳見[圖譜 A/B 量測方案](docs/graph-ab.md)。

**每次 commit 前請檢查 diff。**

<a id="begin"></a>
<a id="install"></a>
<a id="安裝"></a>

## 開始

從 [Releases](https://github.com/xinhi6716/axiom-chronos/releases) 下載安裝包並解壓縮。需要 [Claude Code](https://claude.com/claude-code) 與 Codex CLI，兩者都要先登入。

以下是 3.0.0-beta.1 預先發行版的安裝包；最新穩定版 2.0.1 的安裝包仍在同一頁。

Windows x64：`axiom-chronos-3.0.0-beta.1-windows-x64.zip`。在解壓後的資料夾執行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

Linux x64：`axiom-chronos-3.0.0-beta.1-linux-x64.tar.gz`。在解壓後的資料夾執行：

```sh
sh ./install.sh
```

Windows 把兩個程式裝到 `%LOCALAPPDATA%\Programs\Axiom Chronos`、加入使用者 PATH 與開始功能表捷徑，可原地升級既有安裝；升級前請先關閉兩個程式。從 3.0 起，關閉桌面視窗後背景服務仍會執行，升級前請在系統匣選擇 **Quit**。可用 `-Prefix DIR`、`-NoPath`、`-NoShortcut` 或 `-NoPlugin` 調整。Linux 把 `axiom` 裝到 `~/.local/bin`（需要時自行加入 PATH），可用 `--prefix DIR` 或 `--no-plugin`。兩者在找到 `claude` 時會加入或更新 Claude 的 `chronos` 插件，最後印出 `axiom doctor` 結果；健康檢查有問題不會判定安裝失敗。Windows 附有 `uninstall.ps1`，解除安裝會保留 `~/.axiom` 與 Claude 插件。裸執行檔仍可下載。

或用 Rust 建置：

```sh
cargo install --git https://github.com/xinhi6716/axiom-chronos --tag v3.0.0-beta.1 --locked --bin axiom
axiom doctor
```

在你能執行測試的專案目錄中：

```sh
axiom wish 在 cli.py 加一個 --stats 參數，印出 已完成/總數
```

在 Claude Code 裡，用 `/chronos:wish` 加上同樣的文字。

**先保留版本檢查點。** 願望會修改檔案，並把專案資料送到你已登入的模型服務。

<a id="desktop-app"></a>

## 桌面應用程式

桌面應用程式僅支援 Windows，不用開終端機就能許願、觀看並調整進行中的工作。用 `chronos` 或開始功能表捷徑啟動。啟用 `app` feature 的建置也能執行 `axiom app`。想自行建置桌面程式，可執行 `cargo install --git https://github.com/xinhi6716/axiom-chronos --tag v3.0.0-beta.1 --locked --features app --bin chronos`。 底欄隱藏新鮮方案快照未提供的額度視窗，並以天體向量圖符顯示各模型即時並行數；迷你視窗空間不足時只顯示並行總數。

主要畫面：**Run**（目標、模組、單元的待辦清單，即時的執行圖，以及每個單元的 brief、報告、驗證輸出與審查）、**Wishes**（過去與進行中的願望，哪一個需要你處理）、**Memory**（Chronos 記得什麼，可更正或刪除條目）、**Usage**（每次執行或每個願望的 token 與費用，也可比較兩個願望）、**Settings**（語言、主題、健康檢查）。3.0 新增 **Storage**（磁碟用量與清理）、**Research**（為含 `research-manifest.json` 的專案提供稽核工作台）、**Write paper**、**Chat**、**Quick task**、**Plaza** 與 **Routine** 模式，以及背景服務所託管 session 的即時頁面。Ctrl+N 新增願望、Ctrl+K 搜尋全部。在終端機啟動的願望也會出現在這裡，但只能檢視。

關閉視窗只會中斷連線：願望繼續在背景服務（`axiom serve`）執行，再次開啟時重新連上。Windows 系統匣提供 **Open**、**Pause all** 與 **Quit**。設定中的 **Start at login** 預設關閉。詳見[儲存空間](docs/storage.md)與[關閉視窗](docs/app-design.md#closing-the-window)。

剩餘額度列顯示 Claude 與 Codex，每個額度時窗各有一個沙漏。可用迷你視窗並選擇保持最上層、聚焦模式，以及舒適或緊湊間距。控制項支援右鍵選單、復原、提示與鍵盤操作；按 `?` 查看按鍵。

摘要看板用 Codex luna 描述每次執行，失敗時改用 Claude Haiku；也可指定 Haiku 或關閉摘要。數量與狀態直接取自紀錄。Haiku 敘述員撰寫最終報告，選用的白話翻譯讓技術文字更易讀。這些模型呼叫會使用你服務的額度。Usage 顯示各模型的 token 與 API 定價等值金額，Codex 採本版記錄的官方 API 費率，並非訂閱帳單。**自動演進**按鈕會預覽近期問題，確認後才開始修改。

<a id="the-work"></a>
<a id="how-a-wish-runs"></a>
<a id="一個願望怎麼跑"></a>
<a id="use"></a>
<a id="使用"></a>

## 一個願望怎麼跑

<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/chronos-process-zh-TW-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/chronos-process-zh-TW-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/chronos-process-zh-TW-dark.svg" />
  <img src="assets/chronos-process-zh-TW.svg" width="1200" alt="流程：Astra 先規劃、critic 第二審、architect 切模組、Codex 施工（terra 實作、sol 整合）、Claude 審查程式，最後由完整測試、CI 與記憶收尾。接受結果前仍需檢查修改。" />
</picture></p>

每次 wish 都先由 Astra 規劃，`chronos:critic` 審查計畫，核心裁決後才開始建置。

architect 把工作切成模組，每個模組有介面規格與驗收測試。Astra 為每個單元指定 Codex 等級（`terra`、`sol` 或 `astra`）與推理強度，記錄在模組圖中；`terra` 是下限，修補單元會往上升一級。模組所依賴的模組驗證完成後就立即開工。每個單元先寫會失敗的測試，審查前再由自我檢查比對介面規格、測試與 diff。

`AXIOM_CODEX_MAX_PARALLEL` 限制共用資料目錄的 Chronos 程序之 Codex 並行回合，預設 48；滿額時等待且仍可取消。設定可儲存桌面許願的上限，額度列與 `axiom doctor` 顯示使用槽數；不計入你自行啟動的 Codex。

在有路由的願望中（architect 會為每份模組圖設定路由），單元審查預設交給 Codex 上的 Astra 審查員，除非政策設定了 `reviewer`（`claude`、`astra-max` 或 `both`）；blocker 與 major 發現會再由 Claude 快速核對一次。`chronos:critic`（Claude）審查計畫，Haiku 敘述員依引擎摘要與核心裁決撰寫最終報告。

關卡由執行檔強制：沒有占用單元不能寫檔、審查沒通過那份檔案的單元不算驗收、沒有最新的完整測試不能結案。

審查員依 blocker／major／minor 分級，自行寫入裁決檔。報告列出需要 smoke 或人工檢查的發現。審查結果可沿用到同層修補（`AXIOM_REVIEW_CARRY=0` 可關閉）；修補鏈兩輪後停止，由核心用 `orch waive` 附理由裁決發現，或將願望列為卡住。

只在開頭提問。執行中你保有插話權（interjection）：在 `axiom wish` 輸入一行並按 Enter，就能追加指示、提問或喊停。

剩下的工作若是一大批專案自己的程式，願望會在不呼叫任何模型的情況下等待，批次結束後自動接續。`axiom wish` 會自己等；`axiom wait <session-id>` 用來接手正在等待的願望，例如按過 Ctrl-C 之後。

| 選項 | 用途 |
|---|---|
| `--project PATH` | 處理另一個專案。 |
| `--resume <session-id> [訊息]` | 接續中斷或卡住的願望；若附上訊息，它會是接續後的第一句話。 |
| `--creative` | 讓 Chronos 自己找一個有證據支持的小改進。 |
| `--max-turns N` | 限制核心 session 回合數。 |
| `--need-space SIZE` | 覆寫願望啟動前檢查的磁碟空間估計，例如 `20G`。 |
| `--advanced-research` | 規劃前開啟需付費的研究議會。 |

選項放在願望**前面**，後面的文字都是願望內容。`axiom research` 會把問題變成審查過的研究計畫。

其他指令：`axiom review <git range>` 讓另一家模型對一份 diff 給第二意見（先由 Codex 審，agy 是選用的備用審查員，絕不會卡住流程；`--diff-only` 只送 diff）、`axiom lint` 檢查專案記憶有沒有過時或重複的條目與對不上的引文、`axiom usage` 印出每次執行的 token 與費用明細。

3.0 新增 `axiom do`（執行一個限定範圍的快速任務，見[模式](docs/modes.md)）、`axiom storage`（磁碟用量與清理，見[儲存空間](docs/storage.md)）、`axiom graph` 與 `axiom dream`（原生程式碼與記憶圖），以及 `axiom stats`（審查數量與比率）。

`axiom evolve --dry-run` 從 Chronos 自己最近七天的紀錄產生自我改進摘要。檢視後，再明確執行 `axiom evolve --repo PATH`，對 Chronos 原始碼儲存庫進行改進。這是手動啟動的每週檢查，不是排程工作：桌面會詢問確認；CLI 啟動指令沒有確認提示。

**選用項目。** agy（備用審查員）與 [CodeGraph](https://github.com/colbymchenry/codegraph)（有 `.codegraph/` 資料夾時減少讀碼量）有安裝才會用。`--advanced-research` 會額外花錢：每一輪由 Astra、Fable、Argon（Gemini 4）依序討論計畫，有異議時也依相同順序答辯，最後核心依各階段紀錄裁決。設定 `AXIOM_COUNCIL_ARGON_EVERY_ROUND=0`，或在 `<home>/settings/council.json` 設定 `argon_every_round`，可改回 Astra 與 Fable 對話後由 Argon 只審查一次。不主動開啟就不會用。預設 Fable $5、Astra 200k tokens、Argon 各次呼叫合計 400k tokens，兩個對話階段各最多 3 輪。可設定 `AXIOM_FABLE_CAP_USD`、`AXIOM_ASTRA_TOKEN_CAP`、`AXIOM_ARGON_TOKEN_CAP`、`AXIOM_COUNCIL_ROUNDS`，或到「設定 → 進階研究預算」調整。CLI 可用 `axiom wish --advanced-research --fable-cap 20 --astra-cap 600000 --council-rounds 3 "修正問題"` 覆寫；這三個選項都必須搭配 `--advanced-research`。數值須為正數，token 與輪數須為整數；無效值拒絕啟動，Fable 超過 $200 視為可能誤植而拒絕。達成共識會提早結束；預算用盡、失敗或輪數用完時，把未解爭議交給核心。失敗或預算用盡的成員會標示為降級。Write paper、Astra 裁決者、PM/QA 審查員與雙核心模式也會額外呼叫模型；後三者預設關閉（見[選用與費用](docs/releases/3.0.0-beta.1.md#選用與費用)）。

為了省 token，每個模組各有一個調度員，達交棒門檻的 80% 時先提醒，150k token 時換新調度員。壓縮後會逐字還原約束規則；少用的提示章節需要時才載入。交棒備註會檢查是否含有隱瞞問題的指示。無進展防護會在連續三次接續都沒有進展時停止願望，避免反覆提示（`AXIOM_IDLE_CONTINUES` 可改次數）。

Claude 回報用量上限並附上重置時間時，`axiom wish` 會印出接續時刻、等到重置後自動接續同一個 session；按 Ctrl-C 可中止等待。

<a id="record-review"></a>
<a id="state--review"></a>
<a id="狀態與檢查"></a>

## 紀錄

狀態存在 `~/.axiom`（可用 `AXIOM_HOME` 改位置）：`orchestrate/` 放工作紀錄與 brief，`memory/` 放可搜尋的記憶，`logs/` 放原始 session 事件。這些可能含有專案資料，分享前請先檢查。

<a id="horizon"></a>
<a id="status"></a>
<a id="進度"></a>

## 狀態

**3.0.0-beta.1 是 GitHub 預先發行版（pre-release）。** 最新穩定版為 2.0.1。

目前提供：`chronos`（Windows 桌面應用程式）、`axiom wish`、`axiom wait`、`axiom research`、`axiom do`、`axiom modes`、`axiom review`、`axiom stats`、`axiom approval`、`axiom adjudication`、`axiom lint`、`axiom usage`、`axiom evolve`、`axiom graph`、`axiom dream`、`axiom storage`、`axiom serve`、`axiom workshop`、`axiom extreme`、`axiom research-budget`、`axiom laya`、`axiom doctor`。測試用腳本替身模擬 Claude 與 Codex 來檢查協定；真實模型的表現依專案而異。

**已知限制。** 桌面應用程式正在改用共用元件與資料層：Wishes、側邊面板、Run、各模式頁面、視窗外殼、迷你視窗與覆蓋層仍在移轉，最終截圖與效能關卡尚未執行。代理定義設有 `autoCompactWindow`，但尚未在真實執行中觀察到效果。交棒備註防護也尚未見於真實執行；它使用模式清單，可能漏掉換句話說的指示。單元狀態來自證據，不取決於備註。桌面應用程式僅支援 Windows。完整清單見[版本說明](docs/releases/3.0.0-beta.1.md#本版已知限制)。

[Bridge 技術文件](docs/bridge.md) · [派工規則](engine/DISPATCH.md) · [Wish 流程](engine/skills/wish/SKILL.md) · [版本說明](docs/releases/3.0.0-beta.1.md) · [更新紀錄](CHANGELOG.md) · [桌面應用程式設計](docs/app-design.md)

---

[MIT 授權](LICENSE) · [來源與授權](licenses/) · [素材聲明](assets/ASSET_NOTICES.txt)
