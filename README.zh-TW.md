# Axiom Chronos

[English](README.md) · [繁體中文](README.zh-TW.md)

對你的程式碼許願。Axiom Chronos 用一支小型的模型團隊完成它：Claude Code 統籌、Codex 寫程式、Gemini 用白話說明結果，每個完成的部分都會被記住，留給下一個願望使用。

```
axiom wish 在 cli.py 加一個 --stats 參數，印出 已完成/總數
```

## 一個願望怎麼跑

1. **Opus（核心）**：讀這個專案的歷史，把願望交給調度代理，自己不動程式碼。
2. **調度代理（Sonnet）**：把願望拆成 unit，每個 unit 一份只有 GOAL、SCOPE、VERIFY 的 brief，並貼上記憶裡用得到的內容。
3. **Codex worker（terra）**：在用完即丟的 thread 裡施工，不載入任何插件或 skill，沙箱限制在專案內。只有全專案測試失敗時才派 sol 整合。
4. **驗證**：以指令紀錄為準，不採信 worker 的自我回報。
5. **Gemini** 把 worker 的報告翻成白話；同時 **記憶代理（Haiku）** 記下之後可能重用的東西。
6. **Opus** 用事實核對 Gemini 的翻譯，再回答你。

每個 Claude session 只載入 Axiom 自己的引擎，你的其他 Claude Code 插件、skill 和 hooks 都不會進來。

## 安裝

需要 Python 3.10 以上、[Claude Code](https://claude.com/claude-code)、Codex CLI、[Bun](https://bun.sh)；Antigravity（`agy`）是選配，用來產生白話報告。請先登入 `claude` 和 `codex`。

```
pipx install git+https://github.com/xinhi6716/axiom-chronos
axiom doctor
```

`axiom doctor` 會檢查所有依賴並告訴你缺什麼。Windows 上，Codex 無法透過 Microsoft Store 版的 PowerShell 執行指令，doctor 會指出來並給你 MSI 版的安裝指令（[細節](docs/bridge.md)）。

## 使用

```
cd 你的專案
axiom wish 幫 todo 加一個統計功能
axiom wish --project ../other --model sonnet 把 calc 全部改名成 compute_total
```

選項要寫在願望前面，之後的所有文字（包括 `--pending` 這種參數名稱）都算願望內容。如果願望留下了會改變介面或資料的選擇（公開函式、參數、輸出或檔案格式），它會停下來問你，而不是自己猜。

狀態放在 `~/.axiom`（可用 `AXIOM_HOME` 改位置）：每次執行的 orch store（含所有 brief 和 worker 原始報告）、長線記憶，以及每次執行的原始事件紀錄。

## 進度

0.1.0 提供 `axiom wish` 和 `axiom doctor`。接下來會把流程的其他階段做成指令（`ask`、`review`、`research`、`catchup`、`evolve`），並把 [axiom-forge](https://github.com/xinhi6716/axiom-forge) 的 65 個 skill 整合進各階段，而不是一份清單。

## 致謝

工作追蹤使用 poteto 的 [Pstack](https://github.com/cursor/plugins/tree/main/pstack) 中的 `orch`（MIT），原封不動放在 `src/axiom/engine/vendor/pstack`。詳見 [licenses](licenses/)。

MIT 授權。
