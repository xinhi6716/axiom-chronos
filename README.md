# Axiom Chronos

[English](README.md) · [繁體中文](README.zh-TW.md)

> **Beta.** Axiom Chronos is in testing. Commands, state layout and agent behaviour may change between releases, and the Codex build step has so far been verified on small projects only. Review what a wish changed before you commit it.

Make a wish on your codebase. Axiom Chronos runs it as a small team of models: Claude Code coordinates, Codex writes the code, Gemini explains the outcome in plain words, and every finished piece is remembered for the next wish.

```
axiom wish add a --stats flag to cli.py that prints done/total
```

## How a wish runs

```
axiom wish ──► Claude Code (Opus, core)
                 │  reads the project's history
                 ▼
               dispatcher (Sonnet)
                 │  splits the wish into units: GOAL · SCOPE · VERIFY
                 │  pastes what memory knows into each brief
                 ▼
               Codex workers (terra; sol integrates only when the full suite fails)
                 │  ephemeral threads, no plugins or skills, sandboxed to the project
                 ▼
               verification from the command record, never from a worker's claim
                 │
        ┌────────┴─────────┐
        ▼                  ▼
  Gemini rewrites     memory keeper (Haiku)
  reports in plain    records what later
  language            units can reuse
        └────────┬─────────┘
                 ▼
            Opus checks the rewrite against the facts and answers you
```

Each Claude session loads only Axiom's own engine: your other Claude Code plugins, skills and hooks stay out of it.

## Install

Requires Python 3.10+, [Claude Code](https://claude.com/claude-code), the Codex CLI, [Bun](https://bun.sh), and optionally Antigravity (`agy`) for plain-language reports. Sign in to `claude` and `codex` first.

```
pipx install git+https://github.com/xinhi6716/axiom-chronos
axiom doctor
```

`axiom doctor` checks every dependency and says what to fix. On Windows, Codex cannot run commands through a Microsoft Store build of PowerShell; doctor flags it and gives the MSI install command ([details](docs/bridge.md)).

## Use

```
cd your-project
axiom wish 幫 todo 加一個統計功能
axiom wish --project ../other --model sonnet rename calc to compute_total everywhere
```

Put options before the wish; everything after them is the wish, flags included. A wish that leaves open a choice about an interface or data (a public function, a flag, an output or file format) stops with a question instead of guessing.

State lives in `~/.axiom` (set `AXIOM_HOME` to move it): orch stores with every brief and raw worker report, the long-run memory, and a raw event log per run.

## Status

0.1.0b1, a beta, ships `axiom wish` and `axiom doctor`. Next, the rest of the pipeline becomes commands — `ask`, `review`, `research`, `catchup`, `evolve` — and the 65 skills of [axiom-forge](https://github.com/xinhi6716/axiom-forge) are folded into the stages that use them rather than offered as a list.

## Credits

Work tracking runs on `orch` from poteto's [Pstack](https://github.com/cursor/plugins/tree/main/pstack) (MIT), vendored unchanged under `src/axiom/engine/vendor/pstack`. See [licenses](licenses/).

MIT License.
