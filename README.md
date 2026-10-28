<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/readme-cover-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/readme-cover-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme-cover-dark.svg" />
  <img src="assets/readme-cover.svg" width="1200" alt="Axiom Chronos — public beta, with an original celestial time-axis emblem" />
</picture></p>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-TW.md">繁體中文</a><br /><a href="#begin">Begin</a> · <a href="#the-work">The work</a> · <a href="#record-review">Record &amp; review</a> · <a href="#horizon">Horizon</a></p>

Chronos turns a code-change request into scoped work, recorded checks and useful context for the next wish.

> **Beta · 0.1.0b1.** Commands, state and agent behaviour may change. The initial project report describes small-project Codex runs only; the repository tests use scripted stand-ins. Review every change before committing.

<a id="begin"></a>
<a id="install"></a>

## I · Begin

Start small, in a project whose tests you can run.

Requires Python 3.10+, [Claude Code](https://claude.com/claude-code), the Codex CLI and [Bun](https://bun.sh). Sign in to `claude` and `codex` first.

```sh
pipx install git+https://github.com/xinhi6716/axiom-chronos
axiom doctor
```

Then, from your project directory:

```sh
axiom wish add a --stats flag to cli.py that prints done/total
```

**Keep a checkpoint first.** A wish can edit files and call your signed-in model services. Use only project material you are allowed to share with those services.

<details>
<summary><strong>Setup notes</strong> — optional reports and Windows</summary>

`axiom doctor` checks tools, SQLite memory support and the state directory; it does not run a wish. Antigravity (`agy`) is optional. Without its Gemini preview, the core can show the dispatcher report.

The package declares Python 3.10+. CI runs Python 3.12; Python 3.10 runtime compatibility has not been independently tested. SQLite must support FTS5 and JSON.

On Windows, Codex cannot run commands through a Microsoft Store build of PowerShell. Doctor flags it and gives the MSI install command. See [the bridge notes](docs/bridge.md).

</details>

<a id="use"></a>

<details>
<summary><strong>Options &amp; more examples</strong></summary>

```sh
cd your-project
axiom wish add a statistics command to todo
axiom wish --project ../other --model sonnet rename calc to compute_total everywhere
```

Put options **before** the wish. Everything after them is wish text, including flag names.

| Option | Default | Purpose |
| --- | --- | --- |
| `--project PATH` | Current directory | Choose the project. |
| `--model MODEL` | `opus` | Choose the core Claude model. |
| `--max-turns N` | `60` | Limit core-session turns. |

</details>

<a id="the-work"></a>
<a id="how-a-wish-runs"></a>

## II · The work

A brief names the **goal**, the files in **scope**, and the command that will **verify** the result. The dispatcher is instructed to ask first when an interface or data choice is still open.

<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/chronos-process-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/chronos-process-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/chronos-process-dark.svg" />
  <img src="assets/chronos-process.svg" width="1200" alt="Designed beta flow: scope the work, build in Codex threads, check command evidence, and remember useful verified work. Review changes before accepting them." />
</picture></p>

<details>
<summary><strong>Who does what</strong> — roles and boundaries</summary>

Claude Code coordinates. Codex builds. Optional Gemini reports explain what happened.

| Role | Work |
| --- | --- |
| Claude Code core · Opus by default | Reads project history, delegates the wish and checks the final explanation against the reported facts. |
| Dispatcher · Sonnet | Writes `GOAL · SCOPE · VERIFY` briefs and includes relevant memory. It checks the command record or reruns VERIFY. |
| Codex workers · `terra` role | Works in ephemeral threads. A `sol` integration unit is used only if the full suite fails after more than one unit changed files. |
| Optional Gemini via `agy` | Rewrites reports in plain language. If the preview fails, the core shows the dispatcher report. |
| Memory keeper · Haiku | Saves reusable work from verified units for later wishes. |

Claude sessions load Axiom's engine with local settings. Codex workers use lean mode by default, requesting that unrelated plugins, MCP servers, apps and skills context be disabled for that thread. See [bridge contracts](docs/bridge.md#contracts).

</details>

<a id="record-review"></a>
<a id="state--review"></a>

## III · Record &amp; review

**Read the diff. Check what VERIFY ran. Resolve open questions.**

A worker's claim is not evidence, and a completed model session does not guarantee a correct change. Look for failed units before you accept the result.

State stays in `~/.axiom`; use `AXIOM_HOME` to choose another location.

<details>
<summary><strong>What is kept</strong> — work records, memory and logs</summary>

```text
~/.axiom/
├── orchestrate/   Per-project work records, briefs and raw reports
├── memory/        Searchable memory and project-history summaries
└── logs/          Raw session events, one log per run
```

Memory retains the wish and hashes of referenced files, so later lookups can flag stale content. Logs and reports may contain project material; inspect them before sharing.

</details>

<a id="horizon"></a>
<a id="status"></a>

## IV · Horizon

**Here now:** `axiom wish` · `axiom doctor`

**Still ahead:** `ask` · `review` · `research` · `catchup` · `evolve`

The roadmap includes bringing [Axiom Atropos](https://github.com/xinhi6716/axiom-atropos) skills into the stages that use them. These commands and integrations are future work, not shipped beta features.

<details>
<summary><strong>Explore the engine &amp; verification</strong></summary>

[Bridge reference](docs/bridge.md) · [Dispatch policy](src/axiom/engine/DISPATCH.md) · [Wish workflow](src/axiom/engine/skills/wish/SKILL.md) · [Tests](tests/)

The Python tests use scripted stand-ins for Claude, Codex and `agy`. Passing them checks protocol and local behaviour, not live model performance or full end-to-end reliability.

</details>

---

Work tracking uses `orch` from poteto's [Pstack](https://github.com/cursor/plugins/tree/main/pstack), vendored unchanged under `src/axiom/engine/vendor/pstack`. [Credits &amp; licenses](licenses/).

[MIT License](LICENSE) · Original Axiom Celestial Order identity · [Asset notices](assets/ASSET_NOTICES.txt)
