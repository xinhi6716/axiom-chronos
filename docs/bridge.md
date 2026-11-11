<p><a href="../README.md">Axiom Chronos</a> / Engine reference · <a href="../README.zh-TW.md">繁體中文概覽</a></p>

# The bridge

<p><a href="#tools">Tools</a> · <a href="#contracts">Contracts</a> · <a href="#configuration">Configuration</a> · <a href="#tests">Tests</a></p>

`axiom bridge` runs an MCP stdio server that lets Claude Code drive Codex through typed tools instead of ad-hoc shell calls. The engine's plugin manifest registers it as `bridge`, so its tools appear as `mcp__plugin_chronos_bridge__<tool>`. Source: [engine/bridge](../engine/bridge).

<a id="tools"></a>

## I · Tools

| Tool | Backend | Use |
| --- | --- | --- |
| `codex_run` | one long-lived `codex app-server` (JSON-RPC over stdio), one ephemeral thread per call | Send a brief to a `terra` worker or a `sol` integrator; the role resolves to a model the signed-in account lists |
| `orch` | in process | Units, inbox, gates and status for one program, a bounded `report` digest for the core and a `review-pack` of each unit's contract and diff for the reviewer; the store lives under `CLAUDE_PLUGIN_DATA` (or `AXIOM_DATA_HOME`) so it survives plugin updates |
| `memory_record`, `memory_search`, `memory_get`, `memory_digest` | SQLite FTS5 (trigram) in the same data directory | Long-run memory: verified units distilled for later briefs, and a one-line-per-wish history per project. Original-wish text remains searchable even when summaries are English. CJK query runs become bigrams; matching uses stored terms, not translation. Each entry hashes its files and reports `stale` once they change |
| `bridge_status` | `--version` probe | Resolved Codex executable, whether the app-server is up, and how the installed Codex compares with the versions Chronos has been tested with (`codex.version_check`) |

`axiom wish` runs the [wish skill](../engine/skills/wish/SKILL.md) in a headless Claude Code session. The [dispatcher agent](../engine/agents/dispatcher.md) (`chronos:dispatcher`, Sonnet) splits the wish into units, pastes relevant memory into each brief, dispatches terra workers, verifies from the command record, and only calls a sol integrator when the full suite fails. The [memory keeper](../engine/agents/memory-keeper.md) (`chronos:memory-keeper`, Haiku) then records what later units can reuse.

Every wish runs Astra first; no option or marker skips it.

Reviews go to Claude: `chronos:reviewer` reviews one whole unit per call and `chronos:critic` reviews the Astra plan; `chronos:narrator` (Haiku) writes the final report from the run digest and the core's rulings.

Every result carries `status`; anything other than `completed` carries `error.kind` (for example `usageLimitExceeded`, `timeout`, `codex_exited`, `codex_protocol`).

<a id="contracts"></a>

## II · Contracts

- Unit mode takes exactly `project`, `unit`, `role` and `cwd`, plus `track` and `brief_file` to register a new unit in the same call; brief mode is for Astra.
- A registered brief may carry `EFFORT` (`low`, `medium` or `high`) and `TIER` (`luna`, terra units only) as their own lines between GOAL and CONTEXT. The bridge applies them and removes them from the text the worker gets. Any other value, or either line anywhere else in the brief, is refused with `invalid_argument` by `orch unit add` and again at dispatch. Dispatch also refuses an `effort` argument in unit mode, and `TIER` with role `sol` or a `model` argument. With neither line a unit keeps its role's model and Codex's default effort. If the account lists no model for the tier, the call fails with `model_not_found`; it does not fall back.
- When every unit of a module has passed VERIFY, the unit reply's `accept` carries the module ACCEPT result.
- `brief` follows the [dispatch policy](../engine/DISPATCH.md): GOAL, SCOPE and VERIFY, decided before dispatch. The worker sees nothing else. Map a returned report onto `orch inbox push` the same way as any other worker completion.
- A workspace-writing `codex_run` without a bound project and unit is refused with `unbound_write` before Codex starts.
- `cwd` must be an existing absolute directory; give each writer its own worktree.
- `sandbox` is `read-only` or `workspace-write`. The bridge never exposes `danger-full-access`, and `approvalPolicy` is fixed to `never`; stray approval requests are refused and listed in `refused_requests`.
- Codex threads are ephemeral, so workers keep no memory between calls.
- `lean` (default on) changes that thread only: it asks the app-server which plugins and configured MCP servers are enabled and turns them all off, along with the apps connector, goals, the skills catalog, the personality section and the web search tool (`web_search: true` keeps it). `keep_plugins` exempts named plugins. Remote plugins (marketplaces such as `openai-curated-remote`) ignore that config switch, so lean mode also sends their ids as the turn's `disabledPluginIds`; on an account with one remote MCP server this cut a one-word call from 10,410 to 9,630 input tokens. The report lists what was disabled, which MCP servers actually started, and the thread's token `usage`.
- When the [optional code graph](#code-graph) is active, lean mode leaves that one server running; every other configured server is still turned off.
- Upstream API rejections that Codex labels `other` are reported by their provider code, such as `model_not_found`. Available models depend on the signed-in account; `model/list` is the source of truth.

Codex's base instructions (~3.5k tokens) stay in place because they teach the shell and patch tools.

<details>
<summary><strong>Recorded lean-mode measurements</strong></summary>

Measured with `gpt-5.6-terra` at low effort:

| Brief | Inherited config | Lean |
| --- | --- | --- |
| One-word reply | 19,863 input tokens | 8,457 |
| Write a module and its tests, then run them | 149,414 input tokens, FAIL (detoured into Pstack skills, then sandbox-blocked) | 29,084, PASS |

</details>

<a id="configuration"></a>

## III · Configuration

The bridge resolves `codex` from `PATH`. Override it with a JSON array, e.g. `AXIOM_CODEX_CMD='["/opt/codex/bin/codex"]'`. A missing executable fails the call with `not_installed`. Memory needs SQLite with FTS5 trigram support, which `axiom doctor` checks.

<a id="code-graph"></a>

### Optional code graph

[CodeGraph](https://github.com/colbymchenry/codegraph) stays outside Chronos: nothing is bundled or installed, and no Claude or Codex configuration is written. A run uses it only when a `codegraph` executable is on `PATH` and the project has a `.codegraph/` directory (`codegraph init` creates it). Then, for that run:

- The Claude session gets `codegraph serve --mcp` through `--mcp-config`, with `alwaysLoad` as CodeGraph's setup documents, and `--allowedTools` gains `mcp__codegraph__codegraph_explore`, the one tool the server lists by default.
- Each Codex thread gets the same server with `cwd` set to the thread's directory, and lean mode does not disable it.
- The server runs with `CODEGRAPH_TELEMETRY=0`. A daemon CodeGraph already started for the project without it keeps its own setting until it exits; `codegraph telemetry off` covers every process.

`chronos:architect`, `chronos:dispatcher` and `chronos:reviewer` list that tool; Claude Code drops the entry when the server is absent. If either condition is missing, launch arguments and thread settings are exactly what they were without this feature. `axiom doctor` reports what it found and never fails because of it. A server that does not start only leaves its tool unavailable; the run continues without it. No benchmark has measured a saving inside Chronos yet.

### Codex versions

Chronos is not tied to one Codex build, but it speaks that build's app-server protocol. [`src/codex/version.rs`](../src/codex/version.rs) holds the one record of what has been tested: a minimum and a last-tested version. Raise `last_tested` only after running Chronos on that release.

`axiom doctor` and `bridge_status` read `codex --version` and give the same result:

| Installed Codex | Result |
| --- | --- |
| From the minimum through the last-tested version | ok |
| Newer than the last-tested version | warning; runs are not blocked |
| Older than the minimum, or the version cannot be read | failure |

A failed app-server handshake, a request Codex rejects as malformed, a reply missing a field Chronos needs, and a turn that ends with an unknown status all fail with `error.kind` `codex_protocol`. The message names the installed version and the last-tested version, followed by the start of the raw error. A timeout keeps the kind `timeout` and gains the same note when the installed version is outside the tested range.

### Windows · PowerShell

On Windows, the Codex sandbox cannot start Microsoft Store (MSIX) apps: launching the Store builds of `pwsh` directly fails with `CreateProcessAsUserW failed: 5` every time, while System32 `powershell.exe` and `cmd.exe` work. Codex runs worker commands through `pwsh` when one is on PATH, so a machine whose only PowerShell 7 is the Store build leaves workers unable to run VERIFY; the dispatcher then reruns it, so results stay trustworthy.

The fix is PowerShell 7 from its MSI. winget installs the MSIX build by default and refuses to replace a Store install with an MSI, so remove the Store package first, then:

```powershell
winget install --id Microsoft.PowerShell --exact --installer-type wix
```

Restart Claude Code afterwards so the bridge's Codex app-server sees the new PATH.

<a id="tests"></a>

## IV · Tests

`cargo test` runs the backends and the MCP session against scripted stand-ins in `tests/fakes/`.

## Real-host acceptance checklist

Fake-driven tests are regression checks; real-host acceptance is separate. The following items have **not been verified** on a real Claude Code host:

- [ ] **Not verified — Launch:** Install and launch the Chronos plugin in Claude Code, and start its bridge.
- [ ] **Not verified — Drive:** Run a wish, a `--creative` wish, an interjection, and `--resume`.
- [ ] **Not verified — Observe:** During a long `codex_run`, observe progress notifications; inspect `claims.tsv` and verify the evidence files.
- [ ] **Not verified — Cleanup:** Remove the acceptance run's claims, goals, and temporary worktrees.
