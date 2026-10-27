<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/readme-cover-dark-mobile.svg" />
    <source media="(max-width: 600px)" srcset="assets/readme-cover-mobile.svg" />
    <source media="(prefers-color-scheme: dark)" srcset="assets/readme-cover-dark.svg" />
    <img src="assets/readme-cover.svg" width="1000" alt="Axiom Chronos — beta codebase workflow, with an original celestial time emblem" />
  </picture>
</p>

# Axiom Chronos

**Make a wish on your codebase. Follow what changed.**

[English](README.md) · [繁體中文](README.zh-TW.md)

> **Beta · 0.1.0b1.** Commands, state layout and agent behaviour may change between releases. The Codex build step has so far been verified on small projects only. Review what a wish changed before you commit it.

[Install](#install) · [Use](#use) · [How it works](#how-a-wish-runs) · [State & review](#state--review) · [Roadmap](#status) · [Bridge reference](docs/bridge.md)

Chronos brings a small team of models to a code change: **Claude Code coordinates, Codex builds, and optional Gemini reports explain the result.** A memory keeper saves useful, verified work for later wishes.

```sh
axiom wish add a --stats flag to cli.py that prints done/total
```

## Install

Requires Python 3.10+, [Claude Code](https://claude.com/claude-code), the Codex CLI, [Bun](https://bun.sh), and optionally Antigravity (`agy`) for plain-language reports. Sign in to `claude` and `codex` first.

```sh
pipx install git+https://github.com/xinhi6716/axiom-chronos
axiom doctor
```

`axiom doctor` checks the required tools, SQLite memory support and the state directory, then says what to fix. It does not run a wish. Missing `agy` is a warning rather than a required-dependency failure.

**Windows:** Codex cannot run commands through a Microsoft Store build of PowerShell; doctor flags it and gives the MSI install command. See [the bridge notes](docs/bridge.md).

## Use

Start with a small change in a project whose tests you can run.

```sh
cd your-project
axiom wish 幫 todo 加一個統計功能
axiom wish --project ../other --model sonnet rename calc to compute_total everywhere
```

Put options **before** the wish. Everything after them belongs to the wish, including flag names.

| Option | Purpose |
| --- | --- |
| `--project PATH` | Choose the project directory; defaults to the current directory. |
| `--model MODEL` | Choose the core Claude model; defaults to `opus`. |
| `--max-turns N` | Set the core session's turn limit; defaults to `60`. |

The dispatcher is instructed to ask before dispatching if a wish leaves an interface or data choice open, such as a public function, command-line flag, output or file format.

**A wish can edit your project and call your signed-in model services.** Use material you are allowed to share with those services, keep a clean checkpoint, and review the diff and test results before committing.

## How a wish runs

The beta engine separates the work into a few clear roles.

| Step | Role | What it does |
| --- | --- | --- |
| **01 · Remember** | Claude Code core · Opus by default | Reads the project's history and hands the wish to the dispatcher. |
| **02 · Scope** | Dispatcher · Sonnet | Writes a `GOAL · SCOPE · VERIFY` brief for each unit and includes relevant memory. |
| **03 · Build** | Codex workers · `terra` role | Works in ephemeral threads. A `sol` integration unit is used only if the full suite fails after more than one unit changed files. |
| **04 · Check** | Dispatcher | Checks that VERIFY ran successfully in the command record, or reruns it; a worker's claim is not evidence. |
| **05 · Explain & retain** | Gemini via optional `agy` + memory keeper · Haiku | Rewrites reports in plain language and saves reusable verified work. If the preview fails, the core shows the dispatcher report. |
| **06 · Answer** | Claude Code core | Checks the explanation against the reported facts and returns the result. |

Each Claude session is configured to load Axiom's own engine with local settings. Codex workers use lean mode by default, which requests that unrelated plugins, MCP servers, apps and skills context be disabled for that thread. See [bridge contracts](docs/bridge.md#contracts) for the implementation boundaries.

## State & review

State lives in `~/.axiom`; set `AXIOM_HOME` to move it.

```text
~/.axiom/
├── orchestrate/   Per-project work records, briefs and raw reports
├── memory/        Searchable memory and project-history summaries
└── logs/          Raw session events, one log per run
```

Memory entries retain the wish and record hashes of the referenced files so later lookups can flag stale content. Logs and reports may contain project material; inspect them before sharing.

**Before you accept a change:** read the diff, check what VERIFY actually ran, and look for failed units or unanswered questions. A completed model session is not a guarantee that the requested change is correct.

## Status

| Available in this beta | Planned, not available as commands yet |
| --- | --- |
| `axiom wish` · `axiom doctor` | `ask` · `review` · `research` · `catchup` · `evolve` |

The roadmap also includes folding the 65 skills of [axiom-forge](https://github.com/xinhi6716/axiom-forge) into the stages that use them. Those integrations are future work, not part of this beta's command set.

## Explore the engine

[Bridge reference](docs/bridge.md) · [Dispatch policy](src/axiom/engine/DISPATCH.md) · [Wish workflow](src/axiom/engine/skills/wish/SKILL.md) · [Tests](tests/)

The Python tests use scripted stand-ins for Claude, Codex and `agy`. Passing those tests checks protocol and local behaviour; it does not establish live model performance or full end-to-end reliability.

## Credits

Work tracking runs on `orch` from poteto's [Pstack](https://github.com/cursor/plugins/tree/main/pstack) (MIT), vendored unchanged under `src/axiom/engine/vendor/pstack`. See [licenses](licenses/).

**[MIT License](LICENSE).** The original Chronos identity follows Axiom's Celestial Order visual language. [Asset notices](assets/ASSET_NOTICES.txt).
