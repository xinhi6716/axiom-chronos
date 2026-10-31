<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/readme-cover-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/readme-cover-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme-cover-dark.svg" />
  <img src="assets/readme-cover.svg" width="1200" alt="Axiom Chronos — 程式碼願望流程測試版，以原創天體時間軸呈現" />
</picture></p>

<h1 align="center">Axiom Chronos</h1>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-TW.md">繁體中文</a></p>

<p align="center"><a href="#begin">開始</a> · <a href="#the-work">使用</a> · <a href="#status">狀態</a></p>

用僅依賴 Codex 的 Rust 執行環境協作處理範圍明確的程式修改，保留驗證紀錄與可重用的專案內容。

**公開測試版（beta）· 0.2.2-codex.dev.7。** 每次 commit 前請檢查 diff。

| 01 · 準備 | 02 · 描述 | 03 · 檢查 |
| --- | --- | --- |
| 用 `axiom doctor` 檢查環境。 | 交給 `axiom wish` 一項具體修改。 | 檢查 diff 與實際驗證紀錄。 |

<a id="begin"></a>
<a id="install"></a>
<a id="安裝"></a>

## 開始

需要 Codex CLI，並登入可使用所需模型的帳號。在此 Codex-only checkout 的根目錄使用 Rust 從本機建置安裝單一 `axiom` 執行檔：

```sh
cargo install --path . --locked
axiom doctor
```

若要在 Codex 內使用原生 wish／research 技能，安裝 `axiom` 後，在此儲存庫根目錄執行：

```sh
codex plugin marketplace add . --json
codex plugin add chronos@axiom-chronos-codex --json
```

這會註冊 Codex 包裝技能；原有的 `axiom` CLI 仍可獨立使用。

在你能執行測試的專案目錄中：

```sh
axiom wish 在 cli.py 加一個 --stats 參數，印出 已完成/總數
```

**先保留版本檢查點。** 願望可能修改檔案並呼叫已登入的模型服務。請只使用允許提供給這些服務的專案資料。

<details>
<summary><strong>環境與安裝補充</strong></summary>

`axiom doctor` 檢查工具、SQLite 功能與狀態目錄，不會執行願望。SQLite 需支援 FTS5 與 JSON。

`gemini_preview` 是 Codex 角色的相容名稱，不需要 Gemini 或 Antigravity。

Windows 的 doctor 會指出 Microsoft Store 版 PowerShell 的問題，因為 Codex 無法用它執行 worker 指令。請依 [bridge 說明](docs/bridge.md)改用 MSI 版。

</details>

<a id="the-work"></a>
<a id="how-a-wish-runs"></a>
<a id="一個願望怎麼跑"></a>
<a id="use"></a>
<a id="使用"></a>

## 使用方式

描述一項具體修改。調度代理編寫 `GOAL · SCOPE · VERIFY` brief，交給 Codex worker，再檢查指令紀錄或執行 VERIFY。核心僅對專案和願望無法解決的重大選擇先提問；後續選擇會留下紀錄。

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
| `--max-turns N` | — | 限制各 host thread 的模型／工具步數；達上限即停止並提供續接指令。 |
| `--resume <session-id>` | — | 繼續先前的願望 session。 |
| `--creative` | — | 探索並選擇有證據支持的小幅改進。 |
| `--no-architect` | 關閉 | 保留輸入旗標；不可略過必要的核准規劃。 |
| `--astra` | 關閉 | 先執行獨立 Astra 規劃與裁定流程。 |

- **目前的 Astra medium 核心：**溝通意圖、最簡通過／不通過複核、提交已驗證輪次；不另開核心。
- **Terra high 調度者：**負責路由、持久化工作與一般修復。
- **Astra MAX 研究者：**獨立思考與研究；核心通過後取消研究執行緒訂閱並釋放 Host 上下文索引，保留研究產物。
- **Astra HIGH 規劃者：**依核准研究切分模塊、依賴與驗收。
- **Terra 實作者與 Sol HIGH 組裝者：**完成模塊並組裝。
- **獨立 Astra HIGH 驗證者：**核對來源綁定的測試與軟體包證據，只驗證，不改進。
- **Terra medium 記憶模塊；Luna 內容模塊：**記憶由 Terra 維護；Luna 處理授權的非工程文件、圖片與整理。

Rust host 載入嵌入指示、替換相容佔位符，並透過原有 hooks 與唯讀代理 sandbox 執行限制。Codex worker 預設使用 lean mode，要求在該 thread 停用不相關的插件、MCP server、apps 與 skills context。詳見 [bridge contracts](docs/bridge.md#contracts)。

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

測試以腳本替身檢查協定與本機行為。早期小型專案報告及 lean mode 測量屬歷史紀錄，並非此 Codex-only 執行環境的新測量。

[Bridge 技術文件](docs/bridge.md) · [派工規則](engine/DISPATCH.md) · [Wish 流程](engine/skills/wish/SKILL.md) · [測試](tests/)

---

[MIT 授權](LICENSE) · [來源與授權](licenses/) · [素材聲明](assets/ASSET_NOTICES.txt)

工作追蹤以 Rust 在程序內重新實作 poteto 的 [Pstack](https://github.com/cursor/plugins/tree/main/pstack) `orch`。視覺識別為原創 Axiom Celestial Order 圖像。
