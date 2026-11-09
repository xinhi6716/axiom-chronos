<p align="center"><img src="https://raw.githubusercontent.com/xinhi6716/xinhi6716/95b148368534587e5393bc9ef610b99c01ca8e92/assets/identity-v8/chronos.png" width="280" alt="Axiom Chronos: an hourglass beneath a celestial timewheel"></p>

<h1 align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/xinhi6716/xinhi6716/443d48c713f27d52656ad8ea3cb4c5bb9c7dd440/assets/brand-marks/v1/chronos-icon-dark.svg"><img src="https://raw.githubusercontent.com/xinhi6716/xinhi6716/443d48c713f27d52656ad8ea3cb4c5bb9c7dd440/assets/brand-marks/v1/chronos-icon-light.svg" width="40" height="40" alt=""></picture> Axiom Chronos</h1>

<p align="center"><strong>Plan, build and review code changes with Claude and Codex.</strong></p>

<p align="center"><strong>English</strong> · <a href="README.zh-TW.md">繁體中文</a></p>

<p align="center"><a href="docs/releases/1.0.0.md">Release 1.0.0</a></p>

<p align="center"><a href="#begin">Quick start</a> · <a href="#the-work">Workflow</a> · <a href="#record-review">Records</a> · <a href="#status">Status</a></p>

Make a wish on your codebase. Chronos plans it, splits it into modules, has Codex build them, has Claude review them, and keeps working until the change is verified or blocked.

**Review each diff before committing.**

<a id="begin"></a>
<a id="install"></a>

## Start

You need [Claude Code](https://claude.com/claude-code) and the Codex CLI, both signed in. Download `axiom` from [Releases](https://github.com/xinhi6716/axiom-chronos/releases), or build it with Rust:

```sh
cargo install --git https://github.com/xinhi6716/axiom-chronos --locked
axiom doctor
```

In a project whose tests you can run:

```sh
axiom wish add a --stats flag to cli.py that prints done/total
```

Inside Claude Code, use `/chronos:wish` with the same text.

**Save a checkpoint first.** A wish edits files and sends project material to your signed-in model services.

<a id="the-work"></a>
<a id="how-a-wish-runs"></a>
<a id="use"></a>

## How a wish runs

<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/chronos-process-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/chronos-process-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/chronos-process-dark.svg" />
  <img src="assets/chronos-process.svg" width="1200" alt="Workflow: Astra plans, the critic reviews the plan, the architect splits modules, Codex builds (terra builds, sol integrates), Claude reviews the code, then full tests, CI and memory close the loop. Review changes before accepting them." />
</picture></p>

Every wish starts with Astra: Astra plans the change, `chronos:critic` reviews the plan, and the core rules on it before any unit is built.

The architect splits the work into modules with contracts and acceptance tests. The dispatcher sends each unit to a Codex `terra` worker; `sol` steps in only when the full suite fails after several units changed files.

Claude (`chronos:reviewer`, `chronos:critic`) reviews the Codex work; the core writes the final report in plain language.

The binary enforces the gates: no write without a claimed unit, no verified unit without a passing review of the exact files, no finished goal without a current full test.

Questions are asked only at the start. While it runs you keep interjection: in `axiom wish`, type a line and press Enter to add instructions, ask, or stop.

When what remains is a long batch of the project's own programs, the wish waits on it with no model running and continues when the batch ends. `axiom wish` does this by itself; `axiom wait <session-id>` picks up a wish that is waiting, for example after Ctrl-C.

| Option | Purpose |
|---|---|
| `--project PATH` | Work on another project. |
| `--resume <session-id> [message]` | Continue a stopped or blocked wish; the message, if given, is its first turn. |
| `--creative` | Let Chronos find one small, evidence-backed improvement. |
| `--max-turns N` | Limit core-session turns. |

Put options **before** the wish; everything after them is wish text. `axiom research` turns a question into a reviewed research plan.

When Claude reports a usage limit with a reset time, `axiom wish` prints when it will continue, waits for the reset and resumes the same session by itself; Ctrl-C stops the wait.

<a id="record-review"></a>
<a id="state--review"></a>

## Records

State lives in `~/.axiom` (set `AXIOM_HOME` to move it): work records and briefs in `orchestrate/`, searchable memory in `memory/`, raw session events in `logs/`. They may contain project material; inspect them before sharing.

<a id="horizon"></a>
<a id="status"></a>

## Status

Available: `axiom wish`, `axiom wait`, `axiom research`, `axiom doctor`. Tests run the protocols against scripted stand-ins for Claude and Codex; live-model results vary by project.

[Bridge reference](docs/bridge.md) · [Dispatch policy](engine/DISPATCH.md) · [Wish workflow](engine/skills/wish/SKILL.md) · [Release notes](docs/releases/1.0.0.md)

---

[MIT License](LICENSE) · [Credits and licenses](licenses/) · [Asset notices](assets/ASSET_NOTICES.txt)
