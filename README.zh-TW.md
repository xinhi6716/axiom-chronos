<p align="center"><img src="https://raw.githubusercontent.com/xinhi6716/xinhi6716/95b148368534587e5393bc9ef610b99c01ca8e92/assets/identity-v8/chronos.png" width="280" alt="Axiom Chronos: an hourglass beneath a celestial timewheel"></p>

<h1 align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/xinhi6716/xinhi6716/443d48c713f27d52656ad8ea3cb4c5bb9c7dd440/assets/brand-marks/v1/chronos-icon-dark.svg"><img src="https://raw.githubusercontent.com/xinhi6716/xinhi6716/443d48c713f27d52656ad8ea3cb4c5bb9c7dd440/assets/brand-marks/v1/chronos-icon-light.svg" width="40" height="40" alt=""></picture> Axiom Chronos</h1>

<p align="center"><strong>讓 Claude 與 Codex 一起規劃、實作與審查程式修改。</strong></p>

<p align="center"><a href="README.md">English</a> · <strong>繁體中文</strong></p>

<p align="center"><a href="docs/releases/2.0.0.md">版本 2.0.0</a></p>

<p align="center"><a href="#begin">開始</a> · <a href="#desktop-app">桌面應用程式</a> · <a href="#the-work">工作流程</a> · <a href="#record-review">紀錄</a> · <a href="#status">狀態</a></p>

對你的程式碼許一個願望。Chronos 會先規劃、切成模組、交給 Codex 施工、由 Claude 審查，一直做到修改驗證完成或確定卡住為止。

**每次 commit 前請檢查 diff。**

<a id="begin"></a>
<a id="install"></a>
<a id="安裝"></a>

## 開始

需要 [Claude Code](https://claude.com/claude-code) 與 Codex CLI，兩者都要先登入。從 [Releases](https://github.com/xinhi6716/axiom-chronos/releases) 下載 `axiom`，或用 Rust 建置：

```sh
cargo install --git https://github.com/xinhi6716/axiom-chronos --locked
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

Chronos 也有視窗版，不用開終端機就能許願、觀看並調整進行中的工作。用 `chronos` 啟動，或執行 `axiom app`。Windows 版會把 `chronos.exe` 附在 Release；想自己建置，執行 `cargo install --git https://github.com/xinhi6716/axiom-chronos --locked --features app --bin chronos`。

主要畫面：**Run**（目標、模組、單元的待辦清單，即時的執行圖，以及每個單元的 brief、報告、驗證輸出與審查）、**Wishes**（過去與進行中的願望，哪一個需要你處理）、**Memory**（Chronos 記得什麼，可更正或刪除條目）、**Usage**（每次執行的 token 與費用）、**Settings**（語言、主題、健康檢查）。Ctrl+N 新增願望、Ctrl+K 搜尋全部。在終端機啟動的願望也會出現在這裡，但只能檢視。

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

architect 把工作切成模組，每個模組有介面規格與驗收測試。調度代理把每個單元交給 Codex `terra` 實作；只有多個單元改檔後完整測試失敗時，才由 `sol` 整合。

審查由 Claude（`chronos:reviewer`、`chronos:critic`）負責審 Codex 的成果；最後報告由核心直接以白話撰寫。

關卡由執行檔強制：沒有占用單元不能寫檔、審查沒通過那份檔案的單元不算驗收、沒有最新的完整測試不能結案。

只在開頭提問。執行中你保有插話權（interjection）：在 `axiom wish` 輸入一行並按 Enter，就能追加指示、提問或喊停。

剩下的工作若是一大批專案自己的程式，願望會在不呼叫任何模型的情況下等待，批次結束後自動接續。`axiom wish` 會自己等；`axiom wait <session-id>` 用來接手正在等待的願望，例如按過 Ctrl-C 之後。

| 選項 | 用途 |
|---|---|
| `--project PATH` | 處理另一個專案。 |
| `--resume <session-id> [訊息]` | 接續中斷或卡住的願望；若附上訊息，它會是接續後的第一句話。 |
| `--creative` | 讓 Chronos 自己找一個有證據支持的小改進。 |
| `--max-turns N` | 限制核心 session 回合數。 |
| `--advanced-research` | 規劃前開啟需付費的研究議會。 |

選項放在願望**前面**，後面的文字都是願望內容。`axiom research` 會把問題變成審查過的研究計畫。

其他指令：`axiom review <git range>` 讓另一家模型對一份 diff 給第二意見（先由 Codex 審，agy 是選用的備用審查員，絕不會卡住流程；`--diff-only` 只送 diff）、`axiom lint` 檢查專案記憶有沒有過時或重複的條目與對不上的引文、`axiom usage` 印出每次執行的 token 與費用明細。

**選用項目。** agy（備用審查員）與 [CodeGraph](https://github.com/colbymchenry/codegraph)（有 `.codegraph/` 資料夾時減少讀碼量）有安裝才會用。`--advanced-research` 是唯一額外花錢的選項：規劃前由 Astra、Fable 與 Argon 一起討論，Fable 預設上限 $5。不主動開啟就不會用。

為了省 token，每個模組各有一個調度員，達 150k token 時帶著狀態交棒給新的調度員。

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

目前提供：`chronos`（桌面應用程式）、`axiom wish`、`axiom wait`、`axiom research`、`axiom review`、`axiom lint`、`axiom usage`、`axiom doctor`。測試用腳本替身模擬 Claude 與 Codex 來檢查協定；真實模型的表現依專案而異。

[Bridge 技術文件](docs/bridge.md) · [派工規則](engine/DISPATCH.md) · [Wish 流程](engine/skills/wish/SKILL.md) · [版本說明](docs/releases/2.0.0.md) · [更新紀錄](CHANGELOG.md) · [桌面應用程式設計](docs/app-design.md)

---

[MIT 授權](LICENSE) · [來源與授權](licenses/) · [素材聲明](assets/ASSET_NOTICES.txt)
