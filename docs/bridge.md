<p><a href="../README.md">Axiom Chronos</a> / Engine reference · <a href="../README.zh-TW.md">繁體中文概覽</a></p>

# The bridge

<p><a href="#tools">Tools</a> · <a href="#contracts">Contracts</a> · <a href="#configuration">Configuration</a> · <a href="#tests">Tests</a></p>

`axiom bridge` exposes typed tools backed by the Rust host and Codex. The external MCP compatibility tool names remain `mcp__plugin_chronos_bridge__<tool>`. The session host instead exposes legal dynamic aliases `chronos_<tool>` (Codex reserves the `mcp__` prefix), then maps them back to the unchanged bridge and hook names. Source: [src/bridge](../src/bridge).

<a id="tools"></a>

## I · Tools

| Tool | Backend | Use |
| --- | --- | --- |
| `codex_run` | one long-lived `codex app-server` (JSON-RPC over stdio), one ephemeral thread per call | Send a brief to a `terra` worker or a `sol` integrator; the role resolves to a model the signed-in account lists |
| `gemini_preview` | Codex (compatibility alias) | Sol second review or Luna rewrite; stage evidence records the actual model |
| `orch` | in process | Units, inbox, gates and status for one program; the store lives under `CLAUDE_PLUGIN_DATA` (or `AXIOM_DATA_HOME`) so it survives plugin updates |
| `memory_record`, `memory_search`, `memory_get`, `memory_digest` | SQLite FTS5 (trigram) in the same data directory | Long-run memory: verified units distilled for later briefs, and a one-line-per-wish history per project. Original-wish text remains searchable even when summaries are English. CJK query runs become bigrams; matching uses stored terms, not translation. Each entry hashes its files and reports `stale` once they change |
| `review_code` | Codex Sol or Astra | Review all supplied files against the contract using a different engineering role from the writer |
| `run_on_linux` | local Linux / WSL | Run Python verification on Linux when available |
| `goal` | Rust host | Declare mode and close a wish with recorded evidence |
| `bridge_status` | status probes | Report runtime status |

`axiom wish` bootstraps an attachment for the current caller; it never launches a replacement core. The Rust authority workflow is core intent → Terra dispatcher → independent Astra MAX research → core intent verdict → Astra HIGH planning → Terra implementation → Sol HIGH assembly → independent Astra HIGH testing → core submission. Research is unsubscribed on approval and Host context metadata is released. Memory is Terra medium; Luna remains available for nonengineering content.

Only `goal`, `workflow` and dispatcher delegation are exposed to core through an attached bridge. Rust binds caller identity, phase, research revision and source-bound acceptance; an `actor` argument cannot grant privileges. Calls await the dispatcher synchronously, without GOAL auto-continuation. This restricts Chronos operations, not the desktop host's own external tools.

`review_code` selects an engineering model distinct from the actual writer (Sol or Astra). This is distinct-role review within OpenAI Codex, not the original cross-family review. Reviewer fallback records the actual model, not an agent alias. The Rust host enforces the original hooks and read-only sandboxes for core and agents; it parses role model/effort metadata, strips frontmatter and substitutes compatibility placeholders.

Sessions retain durable workflow state. Independent role calls use distinct contexts; research-only answer/review sessions retain plan/review/ruling stages without engineering gates. Terminal transport failure is not successful task completion.

Luna content can read UTF-8 documents and inspect images using native `view_image`, then write UTF-8 reports/documents. The root authorizes exact `output_paths` on its dispatcher Agent call; the dispatcher may pass only a subset to content. This does not grant dispatcher project-write authority. No binary image-generation/editing subsystem is provided.

`--max-turns N` is a conservative per-host-thread model/tool-step budget: each `turn/start` and each dynamic tool callback consumes one step; native tool `item/started` events also consume steps (dynamic lifecycle events are not counted twice). Parallel tool calls count individually. Exhaustion interrupts the active turns, cancels outstanding work and retains the persistent root for resume. This is not an exact count of provider-internal inference turns, which app-server does not expose. Worker calls retain their existing separate timeout contract.

Results report status and errors; model availability depends on the signed-in account.

<a id="contracts"></a>

## II · Contracts

- `brief` follows the [dispatch policy](../engine/DISPATCH.md): GOAL, SCOPE and VERIFY, decided before dispatch. The worker sees nothing else. Map a returned report onto `orch inbox push` the same way as any other worker completion.
- A workspace-writing `codex_run` without a bound project and unit is refused with `unbound_write` before Codex starts.
- `cwd` must be an existing absolute directory; give each writer its own worktree.
- `sandbox` is `read-only` or `workspace-write`. The bridge never exposes `danger-full-access`, and `approvalPolicy` is fixed to `never`; stray approval requests are refused and listed in `refused_requests`.
- Worker/planner threads are ephemeral, so they keep no memory between calls; the root remains persistent. Native delegation (`multi_agent` and `multi_agent_v2`) is disabled on every host and worker thread, including non-lean workers. Host native shell execution is disabled (`shell_tool`, `unified_exec`); native `view_image` remains enabled.
- `lean` (default on) changes that thread only: it asks the app-server which plugins and configured MCP servers are enabled and turns them all off, along with the apps connector, goals, the skills catalog, the personality section and the web search tool (`web_search: true` keeps it). `keep_plugins` exempts named plugins. Remote plugins (marketplaces such as `openai-curated-remote`) ignore that config switch, so lean mode also sends their ids as every turn's `disabledPluginIds`, including continuation and resume; a historical run on an account with one remote MCP server went from 10,410 to 9,630 input tokens for a one-word call; this is not a measurement of the new runtime. The report lists what was disabled, which MCP servers actually started, and the thread's token `usage`.
- Upstream API rejections that Codex labels `other` are reported by their provider code, such as `model_not_found`. Available models depend on the signed-in account; `model/list` is the source of truth.

Codex's base instructions (~3.5k tokens) stay in place because they teach the shell and patch tools.

<details>
<summary><strong>Historical lean-mode measurements (not new runtime measurements)</strong></summary>

Measured with `gpt-5.6-terra` at low effort:

| Brief | Inherited config | Lean |
| --- | --- | --- |
| One-word reply | 19,863 input tokens | 8,457 |
| Write a module and its tests, then run them | 149,414 input tokens, FAIL (detoured into Pstack skills, then sandbox-blocked) | 29,084, PASS |

</details>

<a id="configuration"></a>

## III · Configuration

The bridge resolves `codex` from `PATH`. Override it with a JSON array, e.g. `AXIOM_CODEX_CMD='["/opt/codex/bin/codex"]'`. A missing executable fails the call with `not_installed`. Memory needs SQLite with FTS5 trigram support, which `axiom doctor` checks.

### Windows · PowerShell

On Windows, the Codex sandbox cannot start Microsoft Store (MSIX) apps: launching the Store builds of `pwsh` directly fails with `CreateProcessAsUserW failed: 5` every time, while System32 `powershell.exe` and `cmd.exe` work. Codex runs worker commands through `pwsh` when one is on PATH, so a machine whose only PowerShell 7 is the Store build leaves workers unable to run VERIFY; the dispatcher then reruns it, so results stay trustworthy.

The fix is PowerShell 7 from its MSI. winget installs the MSIX build by default and refuses to replace a Store install with an MSI, so remove the Store package first, then:

```powershell
winget install --id Microsoft.PowerShell --exact --installer-type wix
```

Restart the axiom session afterwards so its Codex app-server sees the new PATH.

<a id="tests"></a>

## IV · Tests

Rewriters and reviewers process only supplied text in an isolated empty directory, with role-specific instructions forbidding tools, filesystem search and edits; structured schemas and actual token usage are preserved.

`cargo test --locked --offline` runs the backends and the MCP session against scripted stand-ins in `tests/fakes/`.
