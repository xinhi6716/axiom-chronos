<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/readme-cover-dark-mobile.svg" />
    <source media="(max-width: 600px)" srcset="assets/readme-cover-mobile.svg" />
    <source media="(prefers-color-scheme: dark)" srcset="assets/readme-cover-dark.svg" />
    <img src="assets/readme-cover.svg" width="1000" alt="Axiom Chronos — 程式碼願望流程測試版，以原創天體時間徽記呈現" />
  </picture>
</p>

# Axiom Chronos

**對程式碼許願，也看清楚每一步改了什麼。**

[English](README.md) · [繁體中文](README.zh-TW.md)

> **測試版 · 0.1.0b1。** 指令、狀態檔結構和代理行為都可能隨版本改變。Codex 施工這一步目前只在小型專案上驗證過。願望完成後，請先檢查它改了什麼再 commit。

[安裝](#安裝) · [使用](#使用) · [運作流程](#一個願望怎麼跑) · [狀態與檢查](#狀態與檢查) · [進度](#進度) · [Bridge 技術文件](docs/bridge.md)

Chronos 用一支小型模型團隊處理程式碼修改：**Claude Code 統籌、Codex 寫程式，選配的 Gemini 報告用白話解釋結果。** 記憶代理會保存已驗證、日後用得上的工作，留給下一個願望。

```sh
axiom wish 在 cli.py 加一個 --stats 參數，印出 已完成/總數
```

## 安裝

需要 Python 3.10 以上、[Claude Code](https://claude.com/claude-code)、Codex CLI、[Bun](https://bun.sh)；Antigravity（`agy`）是選配，用來產生白話報告。請先登入 `claude` 和 `codex`。

```sh
pipx install git+https://github.com/xinhi6716/axiom-chronos
axiom doctor
```

`axiom doctor` 會檢查必要工具、SQLite 記憶功能與狀態目錄，並說明需要修正的地方；它不會執行願望。缺少 `agy` 只會顯示警告，不會被當成必要依賴失敗。

**Windows：** Codex 無法透過 Microsoft Store 版的 PowerShell 執行指令。doctor 會指出來並提供 MSI 版安裝指令，詳見 [bridge 說明](docs/bridge.md)。

## 使用

先從一個小修改開始，並選擇你能執行測試的專案。

```sh
cd 你的專案
axiom wish 幫 todo 加一個統計功能
axiom wish --project ../other --model sonnet 把 calc 全部改名成 compute_total
```

選項要放在願望**前面**。之後的文字全部算願望內容，包括參數名稱。

| 選項 | 用途 |
| --- | --- |
| `--project PATH` | 指定專案目錄，預設為目前目錄。 |
| `--model MODEL` | 指定核心 Claude 模型，預設為 `opus`。 |
| `--max-turns N` | 設定核心 session 的回合上限，預設為 `60`。 |

調度代理的指示要求：若願望留下會改變介面或資料的選擇，例如公開函式、命令列參數、輸出或檔案格式，先向你提問，再派工。

**願望會修改專案，也可能呼叫你已登入的模型服務。** 請只使用允許提供給這些服務的資料，先保留乾淨的版本檢查點，再檢查 diff 與測試結果後 commit。

## 一個願望怎麼跑

測試版引擎把工作分成幾個清楚的角色。

| 步驟 | 角色 | 工作 |
| --- | --- | --- |
| **01 · 回顧** | Claude Code 核心，預設 Opus | 讀取專案歷史，把願望交給調度代理。 |
| **02 · 定義範圍** | 調度代理 · Sonnet | 為每個 unit 寫出 `GOAL · SCOPE · VERIFY`，並加入相關記憶。 |
| **03 · 施工** | Codex worker · `terra` 角色 | 在用完即丟的 thread 裡工作。只有多個 unit 修改了檔案、且全專案測試失敗時，才派 `sol` 做整合。 |
| **04 · 檢查** | 調度代理 | 從指令紀錄確認 VERIFY 確實成功執行，否則重新執行；worker 自稱成功不算證據。 |
| **05 · 說明與記憶** | 選配 `agy` 的 Gemini + 記憶代理 · Haiku | 將報告改寫成白話，保存已驗證、可重用的工作。白話預覽失敗時，核心改顯示調度報告。 |
| **06 · 回答** | Claude Code 核心 | 用回報的事實核對說明，再交回結果。 |

每個 Claude session 設定為載入 Axiom 自己的引擎與 local settings。Codex worker 預設使用 lean mode，要求在該 thread 停用不相關的插件、MCP server、apps 與 skills context。實作邊界見 [bridge contracts](docs/bridge.md#contracts)。

## 狀態與檢查

狀態預設放在 `~/.axiom`，可用 `AXIOM_HOME` 改位置。

```text
~/.axiom/
├── orchestrate/   各專案的工作紀錄、brief 與原始報告
├── memory/        可搜尋記憶與專案歷史摘要
└── logs/          每次執行的原始 session 事件
```

記憶項目保留願望原文，並記錄相關檔案的雜湊值，讓之後的查詢能標示過期內容。紀錄與報告可能含有專案資料，分享前請先檢查。

**接受修改前：** 看過 diff，確認 VERIFY 實際執行了什麼，並留意失敗的 unit 或尚未回答的問題。模型 session 正常結束，不代表修改一定正確。

## 進度

| 這個測試版已提供 | 規劃中，尚未提供為指令 |
| --- | --- |
| `axiom wish` · `axiom doctor` | `ask` · `review` · `research` · `catchup` · `evolve` |

後續也規劃把 [axiom-forge](https://github.com/xinhi6716/axiom-forge) 的 65 個 skill 整合進各階段。這些整合仍是後續工作，不屬於目前測試版的指令集。

## 看看引擎內部

[Bridge 技術文件](docs/bridge.md) · [派工規則](src/axiom/engine/DISPATCH.md) · [Wish 流程](src/axiom/engine/skills/wish/SKILL.md) · [測試](tests/)

Python 測試使用腳本替身模擬 Claude、Codex 與 `agy`。通過這些測試可檢查協定與本機行為，不能據此宣稱真實模型表現或完整端到端可靠性。

## 致謝

工作追蹤使用 poteto 的 [Pstack](https://github.com/cursor/plugins/tree/main/pstack) 中的 `orch`（MIT），原封不動放在 `src/axiom/engine/vendor/pstack`。詳見 [licenses](licenses/)。

**[MIT 授權](LICENSE)。** Chronos 的原創視覺沿用 Axiom 的 Celestial Order 識別系統。[素材聲明](assets/ASSET_NOTICES.txt)。
