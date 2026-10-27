<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/readme-cover-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/readme-cover-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme-cover-dark.svg" />
  <img src="assets/readme-cover.svg" width="1200" alt="Axiom Chronos — 程式碼願望流程測試版，以天體沙漏與建築光環呈現" />
</picture></p>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-TW.md">繁體中文</a><br /><a href="#begin">開始</a> · <a href="#the-work">流程</a> · <a href="#record-review">記錄與檢查</a> · <a href="#horizon">下一步</a></p>

<p align="center">Claude Code 統籌，Codex 施工。<br />選配的 Gemini 報告，用白話說清楚結果。</p>

Chronos 把程式碼修改請求分成範圍明確的工作，留下檢查紀錄，也保存下一個願望用得上的內容。

> **測試版 · 0.1.0b1。** 指令、狀態與代理行為仍可能改變。專案最初的說明只回報了小型專案的 Codex 執行經驗；倉庫測試使用腳本替身。每次修改都請先檢查，再 commit。

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="assets/chronos-threshold-1-dark.svg" /><img src="assets/chronos-threshold-1.svg" width="1200" alt="" /></picture></p>

<a id="begin"></a>
<a id="安裝"></a>

## I · 開始

先從一個小修改開始，選擇你能執行測試的專案。

需要 Python 3.10 以上、[Claude Code](https://claude.com/claude-code)、Codex CLI 與 [Bun](https://bun.sh)。請先登入 `claude` 和 `codex`。

```sh
pipx install git+https://github.com/xinhi6716/axiom-chronos
axiom doctor
```

接著，在專案目錄裡許願：

```sh
axiom wish 在 cli.py 加一個 --stats 參數，印出 已完成/總數
```

**先保留版本檢查點。** 願望可能修改檔案，並呼叫你已登入的模型服務。請只使用允許提供給這些服務的專案資料。

<details>
<summary><strong>安裝補充</strong> — 選配報告與 Windows</summary>

`axiom doctor` 檢查工具、SQLite 記憶功能與狀態目錄，不會執行願望。Antigravity（`agy`）是選配；沒有 Gemini 預覽時，核心可改顯示調度報告。

套件宣告支援 Python 3.10 以上；CI 使用 Python 3.12，尚未獨立驗證 Python 3.10 執行環境。SQLite 必須支援 FTS5 與 JSON。

Windows 上，Codex 無法透過 Microsoft Store 版的 PowerShell 執行指令。Doctor 會指出問題並提供 MSI 安裝指令，詳見 [bridge 說明](docs/bridge.md)。

</details>

<a id="使用"></a>

<details>
<summary><strong>選項與更多範例</strong></summary>

```sh
cd 你的專案
axiom wish 幫 todo 加一個統計功能
axiom wish --project ../other --model sonnet 把 calc 全部改名成 compute_total
```

選項放在願望**前面**。之後的文字都是願望內容，包括參數名稱。

| 選項 | 預設 | 用途 |
| --- | --- | --- |
| `--project PATH` | 目前目錄 | 指定專案。 |
| `--model MODEL` | `opus` | 指定核心 Claude 模型。 |
| `--max-turns N` | `60` | 限制核心 session 回合數。 |

</details>

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="assets/chronos-threshold-2-dark.svg" /><img src="assets/chronos-threshold-2.svg" width="1200" alt="" /></picture></p>

<a id="the-work"></a>
<a id="一個願望怎麼跑"></a>

## II · 流程

每份 brief 定義**目標**、可修改的檔案**範圍**，以及**驗證**結果的指令。若介面或資料的選擇尚未定案，調度代理的指示要求先提問，再派工。

<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/chronos-process-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/chronos-process-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/chronos-process-dark.svg" />
  <img src="assets/chronos-process.svg" width="1200" alt="測試版設計流程：定義範圍、由 Codex thread 施工、檢查指令紀錄、記住已驗證且可重用的工作。接受結果前仍需檢查修改。" />
</picture></p>

<details>
<summary><strong>誰負責什麼</strong> — 角色與邊界</summary>

| 角色 | 工作 |
| --- | --- |
| Claude Code 核心，預設 Opus | 讀取專案歷史、交付願望，並用回報的事實核對最終說明。 |
| 調度代理 · Sonnet | 編寫 `GOAL · SCOPE · VERIFY`，加入相關記憶，再檢查指令紀錄或重新執行 VERIFY。 |
| Codex worker · `terra` 角色 | 在用完即丟的 thread 施工。只有多個 unit 改了檔案、且全專案測試失敗時，才派 `sol` 整合。 |
| 選配 `agy` 的 Gemini | 將報告改寫成白話。預覽失敗時，核心改顯示調度報告。 |
| 記憶代理 · Haiku | 從已驗證的 unit 保存下一個願望用得上的工作。 |

Claude session 載入 Axiom 引擎與 local settings。Codex worker 預設採用 lean mode，要求在該 thread 停用不相關的插件、MCP server、apps 與 skills context，詳見 [bridge contracts](docs/bridge.md#contracts)。

</details>

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="assets/chronos-threshold-3-dark.svg" /><img src="assets/chronos-threshold-3.svg" width="1200" alt="" /></picture></p>

<a id="record-review"></a>
<a id="狀態與檢查"></a>

## III · 記錄與檢查

**看過 diff，確認 VERIFY 執行了什麼，處理尚未回答的問題。**

Worker 自稱成功不是證據；模型 session 結束，也不保證修改正確。接受結果前，請留意是否仍有失敗的 unit。

狀態預設放在 `~/.axiom`，可用 `AXIOM_HOME` 指定其他位置。

<details>
<summary><strong>留下哪些內容</strong> — 工作紀錄、記憶與日誌</summary>

```text
~/.axiom/
├── orchestrate/   各專案的工作紀錄、brief 與原始報告
├── memory/        可搜尋記憶與專案歷史摘要
└── logs/          每次執行的原始 session 事件
```

記憶保留願望與相關檔案的雜湊值，讓後續查詢能標示過期內容。紀錄與報告可能含有專案資料，分享前請先檢查。

</details>

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="assets/chronos-threshold-4-dark.svg" /><img src="assets/chronos-threshold-4.svg" width="1200" alt="" /></picture></p>

<a id="horizon"></a>
<a id="進度"></a>

## IV · 下一步

**目前提供：** `axiom wish` · `axiom doctor`

**仍在規劃：** `ask` · `review` · `research` · `catchup` · `evolve`

後續會將 [Axiom Atropos](https://github.com/xinhi6716/axiom-atropos) 的 skill 納入各階段。這些指令與整合仍是未來工作，不屬於目前已提供的測試版功能。

<details>
<summary><strong>引擎內部與驗證範圍</strong></summary>

[Bridge 技術文件](docs/bridge.md) · [派工規則](src/axiom/engine/DISPATCH.md) · [Wish 流程](src/axiom/engine/skills/wish/SKILL.md) · [測試](tests/)

Python 測試以腳本替身模擬 Claude、Codex 與 `agy`。通過測試可檢查協定與本機行為，不能據此宣稱真實模型表現或完整端到端可靠性。

</details>

---

工作追蹤採用 poteto 的 [Pstack](https://github.com/cursor/plugins/tree/main/pstack) 中的 `orch`，原封不動收錄在 `src/axiom/engine/vendor/pstack`。[致謝與授權](licenses/)。

[MIT 授權](LICENSE) · 原創 Axiom Celestial Order 識別 · [素材聲明](assets/ASSET_NOTICES.txt)
