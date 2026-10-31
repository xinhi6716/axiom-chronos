<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/readme-cover-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/readme-cover-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme-cover-dark.svg" />
  <img src="assets/readme-cover.svg" width="1200" alt="Axiom Chronos — public beta, with an original celestial time-axis emblem" />
</picture></p>

<h1 align="center">Axiom Chronos</h1>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-TW.md">繁體中文</a></p>

<p align="center"><a href="#begin">Start</a> · <a href="#the-work">Use</a> · <a href="#status">Status</a></p>

Coordinate scoped code changes with the Codex-only Rust runtime. Keep verification records and reusable project context.

**Public beta · 0.2.2-codex.dev.7.** Review each diff before committing.

| 01 · Prepare | 02 · Describe | 03 · Review |
| --- | --- | --- |
| Check the environment with `axiom doctor`. | Give `axiom wish` one concrete change. | Inspect the diff and verification record. |

<a id="begin"></a>
<a id="install"></a>

## Start

You need the Codex CLI, signed in to an account with the required models. From the root of this Codex-only checkout, build and install the single `axiom` binary locally with Rust:

```sh
cargo install --path . --locked
axiom doctor
```

Optionally expose wish/research as native Codex skills. From this repository root, after installing `axiom`:

```sh
codex plugin marketplace add . --json
codex plugin add chronos@axiom-chronos-codex --json
```

This registers the Codex wrapper; the same `axiom` CLI remains available.

From a project whose tests you can run:

```sh
axiom wish add a --stats flag to cli.py that prints done/total
```

**Save a checkpoint first.** A wish can edit files and call your signed-in model services. Use only project material you are allowed to share with those services.

<details>
<summary><strong>Setup details</strong></summary>

`axiom doctor` checks tools, SQLite support and the state directory; it does not run a wish. SQLite needs FTS5 and JSON.

The `gemini_preview` name is a compatibility alias for Codex roles; it does not require Gemini or Antigravity.

On Windows, doctor flags Microsoft Store PowerShell because Codex cannot use it to run worker commands. Use the MSI build as described in the [bridge notes](docs/bridge.md).

</details>

<a id="the-work"></a>
<a id="how-a-wish-runs"></a>
<a id="use"></a>

## How to use it

Describe one concrete change. The dispatcher writes a `GOAL · SCOPE · VERIFY` brief, sends it to a Codex worker, then checks the command record or runs VERIFY. The core asks only about material choices the project and wish cannot resolve; later choices are recorded.

Put CLI options **before** the wish. Everything after them is wish text, including flag names.

For interjection, press Esc in interactive mode. In `axiom wish`, type a line and press Enter to send it into the running session: amend instructions, answer a question, or stop. Use `axiom research` for research.

```sh
axiom wish --project ../other rename calc to compute_total everywhere
```

`axiom wish --resume <session-id>` resumes a session; `axiom wish --creative` starts creative exploration.

<details>
<summary><strong>Options and roles</strong></summary>

| Option | Default | Purpose |
|---|---|---|
| `--project PATH` | Current directory | Choose the project. |
| `--max-turns N` | — | Bound model/tool steps per host thread; reaching the bound stops the session with resume instructions. |
| `--resume <session-id>` | — | Resume a previous wish session. |
| `--creative` | — | Explore and select a small evidence-backed increment. |
| `--no-architect` | Off | Retained input flag; cannot bypass mandatory approved planning. |
| `--astra` | Off | Run the independent Astra planning and ruling flow first. |

- **Current Astra medium core:** communicates user intent, returns pass/reject for intent conflicts, and submits a tested round. No second core is launched.
- **Terra high dispatcher:** owns routing, persisted work and ordinary repair.
- **Astra MAX researcher:** develops the technical plan in an independent context, unsubscribed after core approval (Host context metadata is released).
- **Astra HIGH planner:** splits approved work into modules, dependencies and acceptance checks.
- **Terra workers and Sol HIGH assembler:** implement modules and assemble the changes.
- **Separate Astra HIGH tester:** validates actual source-bound test/package evidence; it does not improve the product.
- **Terra medium memory keeper; Luna content:** reusable verified memory belongs to Terra. Luna handles authorized nonengineering documents, images and organization.

The Rust host loads embedded instructions, substitutes compatibility placeholders, and enforces the original hooks and read-only agent sandboxes. Codex workers use lean mode by default, requesting that unrelated plugins, MCP servers, apps and skills context be disabled for that thread. [Bridge contracts](docs/bridge.md#contracts).

<p align="center"><picture>
  <source media="(prefers-color-scheme: dark) and (max-width: 600px)" srcset="assets/chronos-process-dark-mobile.svg" />
  <source media="(max-width: 600px)" srcset="assets/chronos-process-mobile.svg" />
  <source media="(prefers-color-scheme: dark)" srcset="assets/chronos-process-dark.svg" />
  <img src="assets/chronos-process.svg" width="1200" alt="Designed beta flow: scope the work, build in Codex threads, check command evidence, and remember useful verified work. Review changes before accepting them." />
</picture></p>

This diagram describes the intended beta workflow, not an enforced correctness guarantee.

</details>

<a id="record-review"></a>
<a id="state--review"></a>

## Review and keep the record

Read the diff, inspect what VERIFY actually ran and resolve any failed units or open questions. A completed model session does not establish a correct change.

State lives in `~/.axiom`; set `AXIOM_HOME` to use another location.

<details>
<summary><strong>Stored records and privacy</strong></summary>

```text
~/.axiom/
├── orchestrate/   Work records, briefs and raw reports
├── memory/        Searchable memory and project-history summaries
└── logs/          Raw session events, one log per run
```

Memory retains the wish and hashes of referenced files so later lookups can flag stale content. Logs, memory and reports may contain project material. Inspect them before sharing.

</details>

<a id="horizon"></a>
<a id="status"></a>

## Current status

**Available:** `axiom wish`, `axiom research` and `axiom doctor`. Commands, state and agent behavior may change during beta.

**Planned:** `ask`, `review`, `catchup`, `evolve`, and stage-specific [Atropos](https://github.com/xinhi6716/axiom-atropos) skill integration. These are not shipped beta features.

Tests use scripted stand-ins to check protocols and local behavior. Earlier small-project reports and lean-mode measurements are historical, not measurements of this Codex-only runtime.

[Bridge reference](docs/bridge.md) · [Dispatch policy](engine/DISPATCH.md) · [Wish workflow](engine/skills/wish/SKILL.md) · [Tests](tests/)

---

[MIT License](LICENSE) · [Credits and licenses](licenses/) · [Asset notices](assets/ASSET_NOTICES.txt)

Work tracking reimplements poteto’s [Pstack](https://github.com/cursor/plugins/tree/main/pstack) `orch` in Rust, in process. The visual identity is original Axiom Celestial Order artwork.
