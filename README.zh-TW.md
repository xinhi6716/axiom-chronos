<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/readme-cover-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/readme-cover-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme-cover-dark.svg" />
  <img src="assets/readme-cover.svg" width="1200" alt="Axiom Chronos — 程式碼願望流程測試版，以原創天體時間軸呈現" />
</picture></p>

<h1 align="center">Axiom Chronos</h1>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-TW.md">繁體中文</a></p>

<p align="center"><a href="#begin">開始</a> · <a href="#the-work">使用</a> · <a href="#status">狀態</a></p>

用 Claude Code 與 Codex 協作處理範圍明確的程式修改，保留驗證紀錄與可重用的專案內容。

**公開測試版 · 0.2.0-beta.1。** 每次 commit 前請檢查 diff。

| 01 · 準備 | 02 · 描述 | 03 · 檢查 |
| --- | --- | --- |
| 用 `axiom doctor` 檢查環境。 | 交給 `axiom wish` 一項具體修改。 | 檢查 diff 與實際驗證紀錄。 |

<a id="begin"></a>
<a id="install"></a>
<a id="安裝"></a>

## 開始

需要 [Claude Code](https://claude.com/claude-code) 與 Codex CLI；請先登入 `claude` 和 `codex`。下載單一 `axiom` 執行檔，或使用 Rust 建置安裝：

```sh
cargo install --git https://github.com/xinhi6716/axiom-chronos --locked
axiom doctor
```

在你能執行測試的專案目錄中：

```sh
axiom wish 在 cli.py 加一個 --stats 參數，印出 已完成/總數
```

**先保留版本檢查點。** 願望可能修改檔案並呼叫已登入的模型服務。請只使用允許提供給這些服務的專案資料。

<details>
<summary><strong>環境與安裝補充</strong></summary>

`axiom doctor` 檢查工具、SQLite 功能與狀態目錄，不會執行願望。SQLite 需支援 FTS5 與 JSON。

Antigravity（`agy`）是選配。Gemini 預覽失敗時，核心可改顯示調度報告。

Windows 的 doctor 會指出 Microsoft Store 版 PowerShell 的問題，因為 Codex 無法用它執行 worker 指令。請依 [bridge 說明](docs/bridge.md)改用 MSI 版。

</details>

<a id="the-work"></a>
<a id="how-a-wish-runs"></a>
<a id="一個願望怎麼跑"></a>
<a id="use"></a>
<a id="使用"></a>

## 使用方式

描述一項具體修改。調度代理編寫 `GOAL · SCOPE · VERIFY` brief，交給 Codex worker，再檢查指令紀錄或執行 VERIFY。其指示要求遇到未定的介面或資料選擇時先提問。

CLI 選項放在願望**前面**。後面的文字都是願望內容，包括參數名稱。

Interjection（插話）：互動模式按 Esc；使用 `axiom wish` 時輸入一行並按 Enter，即可送入執行中的 session，用於修改指示、回答問題或停止。也可用 `axiom research` 進行研究。

```sh
axiom wish --project ../other 把 calc 全部改名成 compute_total
```

`axiom wish --resume <session-id>` 繼續 session；`axiom wish --creative` 啟動創意探索。

<details>
<summary><strong>選項與角色分工</strong></summary>

| 選項 | 預設 | 用途 |
|---|---|---|
| `--project PATH` | 目前目錄 | 指定專案。 |
| `--max-turns N` | — | 限制核心 session 回合數。 |
| `--resume <session-id>` | — | 繼續先前的願望 session。 |
| `--creative` | — | 探索並選擇有證據支持的小幅改進。 |

- **Claude Code 核心：**讀取專案歷史、交付願望，以回報事實核對最後說明。
- **Sonnet 調度代理：**劃定 unit 範圍、提供相關記憶、核對驗證指令紀錄。
- **Codex `terra` worker：**在用完即丟的 thread 工作。只有多個 unit 改檔且全專案測試失敗時，才派 `sol` 整合。
- **選配 `agy` 的 Gemini：**將報告改寫成白話。
- **Haiku 記憶代理：**保存已驗證 unit 中可重用的工作。

Claude session 載入引擎與 local settings。Codex worker 預設使用 lean mode，要求在該 thread 停用不相關的插件、MCP server、apps 與 skills context。詳見 [bridge contracts](docs/bridge.md#contracts)。

<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/chronos-process-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/chronos-process-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/chronos-process-dark.svg" />
  <img src="assets/chronos-process.svg" width="1200" alt="測試版設計流程：定義範圍、由 Codex thread 施工、檢查指令紀錄、記住已驗證且可重用的工作。接受結果前仍需檢查修改。" />
</picture></p>

圖中是測試版預期流程，不是執行環境強制保證的正確性。

</details>

<a id="record-review"></a>
<a id="state--review"></a>
<a id="狀態與檢查"></a>

## 檢查並保留紀錄

閱讀 diff、確認 VERIFY 實際執行的內容，處理失敗 unit 與未回答的問題。模型 session 結束不代表修改正確。

狀態預設位於 `~/.axiom`；可用 `AXIOM_HOME` 指定其他位置。

<details>
<summary><strong>保存內容與隱私</strong></summary>

```text
~/.axiom/
├── orchestrate/   工作紀錄、brief 與原始報告
├── memory/        可搜尋記憶與專案歷史摘要
└── logs/          每次執行的原始 session 事件
```

記憶保留願望與相關檔案雜湊，讓後續查詢能標示過期內容。日誌、記憶與報告可能含有專案資料，分享前請先檢查。

</details>

<a id="horizon"></a>
<a id="status"></a>
<a id="進度"></a>

## 狀態

**目前提供：**`axiom wish`、`axiom research` 與 `axiom doctor`。測試期間指令、狀態與代理行為仍可能改變。

**仍在規劃：**`ask`、`review`、`catchup`、`evolve`，以及按階段整合 [Atropos](https://github.com/xinhi6716/axiom-atropos) 技能。這些都不是已提供的測試版功能。

測試以腳本替身模擬 Claude、Codex 與 `agy`，檢查協定和本機行為。專案最初的說明只回報小型專案的 Codex 執行經驗；兩者都不證明真實模型表現或完整端到端可靠性。

[Bridge 技術文件](docs/bridge.md) · [派工規則](engine/DISPATCH.md) · [Wish 流程](engine/skills/wish/SKILL.md) · [測試](tests/)

---

[MIT 授權](LICENSE) · [來源與授權](licenses/) · [素材聲明](assets/ASSET_NOTICES.txt)

工作追蹤以 Rust 在程序內重新實作 poteto 的 [Pstack](https://github.com/cursor/plugins/tree/main/pstack) `orch`。視覺識別為原創 Axiom Celestial Order 圖像。
