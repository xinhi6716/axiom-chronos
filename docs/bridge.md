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

Unit reviews go to the Astra reviewer (Codex) by default in routed stores when policy.json has no `reviewer` key; an explicit value wins. `chronos:reviewer` is the Claude policy option or fallback; `chronos:critic` reviews the Astra plan; `chronos:narrator` (Haiku) writes the final report from the run digest and the core's rulings.

Every result carries `status`; anything other than `completed` carries `error.kind` (for example `usageLimitExceeded`, `timeout`, `codex_exited`, `codex_protocol`).

<a id="contracts"></a>

## II · Contracts

- Unit mode takes `project`, `unit`, `role` and `cwd`, optionally `cycle: true`, plus `track` and `brief_file` to register a new unit in the same call; brief mode is for Astra. With `cycle: true`, the reply is an outcome with `next`, `reason`, `verdict`, `review` entries `{unit, pack}`, `paths` and `module_done`, alongside `accept`, `verify_output` and `report`.
- The `approval` bridge tool takes `project` and `op`: `request` also takes `action: {command: [program, ...args], cwd}` and `reason`, returning `id` and `digest`; `status` and `execute` take `id`. Approve and deny are person-only via `axiom approval approve|deny`: use `axiom approval approve|deny <id> --project <project> --digest <digest> [--wish <wish-id>]`, with `--reason <text>` required for denial. CLI approval immediately executes the exact approved command; a bridge `execute` is also available once approved, but an action can be claimed only once. A denial emits `approval_denied` with the action and reason; the attempted correction memory may be refused because its id is not a verified unit.
- The `goal` tool accepts `session` and `classify` (`amend`, `answer`, `stop`, `replan`, or `pause_batch`). For a classified interjection, use the `interjection` key: pass `interjection: {id, applied: {plan_revision, units, scope}}` or `interjection: {id, declined: <reason>}`; exactly one of `applied` and `declined` is required, and `replan` requires a nonempty `plan_revision`. `answer` and `stop` are applied during classification; `pause_batch` holds the batch until resolved. The wish events include `interjection_sent`, `interjection_received`, `interjection_classified`, `interjection_applied` and `interjection_declined`.
- `orch` accepts `policy set` as `policy set <key> <value>`, and `policy show` via its `args`. Keys include `adjudicator` (on/off), `adjudicator.max_adjudications`, `adjudicator.max_usd`, `pm_qa`, `dual_core`, `reviewer` (claude/astra-max/both), and `budget.<role>` (amount or none). Only the person or core may set policy. A role at its budget, or with unknown spend, emits `budget_exceeded` and the call is refused.
- The engine reviews inline; dispatchers do not spawn reviewers. Remaining review entries carry `reviewer` and the unit pack for the selected fallback. PM/QA writes `<store>/reports/<unit>.pm-qa.verdict.json` with the usual verdict format. When the finding verification gate rejects a fix, `orch unit add --fixes` returns `findings_rejected`; no fix unit is created. Do not treat that reply as a fix.
- Goal and orch replies may include `adjudication` (id, decision, reason, evidence, answer) and `person_requests`; council replies use `adjudications` with rulings or a `person` reason. Respect the decision or surface the person request. A person can overturn a ruling with `axiom adjudication overturn <id> --project <project> --reason <text>` (store selection follows the CLI).
- `axiom stats` reports review counts and finding, rejection and escape rates; use `--project <project>`, optionally `--wish <wish-id>` with a project, and `--json` for structured output.
- A registered brief may carry `EFFORT` (`low`, `medium`, `high`, `xhigh`, `max`) and `TIER` (`terra`, `sol`, `astra`) together as lines between GOAL and CONTEXT. Alternatively use `RISK <class>` there. The bridge strips these settings before sending the brief to the worker. Invalid values or positions are refused; TIER astra requires high, xhigh or max. Account model availability comes from model/list; missing models fail rather than falling back.
- `orch` `["module-context", <module>, "--repo", <dir>]` returns bounded module context (16,384 bytes), with `missing` and `incomplete` sections naming sources to consult. `orch` `["map", "--repo", <dir>]` lints the map and reports all problems together, including `NEW_PATHS` and `SHARED_OWNER`.
- A PostToolUse review hook records the reviewer verdict and says `Chronos recorded the review.`; if it says the review was not recorded, fix the cause and call `orch review` yourself.
- When every unit of a module has passed VERIFY, the unit reply's `accept` carries the module ACCEPT result.
- `brief` follows the [dispatch policy](../engine/DISPATCH.md): GOAL, SCOPE and VERIFY, decided before dispatch. The worker sees nothing else. Map a returned report onto `orch inbox push` the same way as any other worker completion.
- A workspace-writing `codex_run` without a bound project and unit is refused with `unbound_write` before Codex starts.
- `cwd` must be an existing absolute directory; give each writer its own worktree.
- `sandbox` is `read-only` or `workspace-write`. The bridge never exposes `danger-full-access`, and `approvalPolicy` is fixed to `never`; stray approval requests are refused and listed in `refused_requests`.
- Codex threads are ephemeral, so workers keep no memory between calls.
- `lean` (default on) changes that thread only: it asks the app-server which plugins and configured MCP servers are enabled and turns them all off, along with the apps connector, goals, the skills catalog, the personality section and the web search tool (`web_search: true` keeps it). `keep_plugins` exempts named plugins. Remote plugins (marketplaces such as `openai-curated-remote`) ignore that config switch, so lean mode also sends their ids as the turn's `disabledPluginIds`; on an account with one remote MCP server this cut a one-word call from 10,410 to 9,630 input tokens. The report lists what was disabled, which MCP servers actually started, and the thread's token `usage`.
- When the [optional code graph](#code-graph) is active, lean mode leaves that one server running; every other configured server is still turned off.
- Upstream API rejections that Codex labels `other` are reported by their provider code, such as `model_not_found`. Available models depend on the signed-in account; `model/list` is the source of truth.

- Each wish has its own store, `<data home>/orchestrate/<project>/<wish id>`, resolved by `orch::wish_store`; a goal without a wish id keeps the project store, and memory and history stay per project.
- `orch` `schedule next|claim|release` uses `<store>/events.jsonl`, with one JSON object per line and gap-free `seq`. `schedule next [--cursor N] [--timeout S]` blocks until a wake kind (`module_claimed`, `module_verified` or `module_released`) after the cursor; S defaults to 300, is capped at 1800, and 0 means no wait. It claims modules whose DEPENDS are all verified and returns `cursor`, `start`, `running`, `verified`, `waiting`, `stuck`, `held`, `done`, `timed_out`. Replaying an old cursor lists the same `start` modules again and never claims twice; a stop or max-turns hold claims nothing. `schedule claim <module>` re-claims a released module; `schedule release <module> --reason <text>` releases it.
- The hook allows `chronos:dispatcher` with `run_in_background: true` only for an engine-claimed module: the goal store has `modules.md` and the target module is claimed. Otherwise it must run in the foreground. Agents may call `schedule release` and `activity`, but not `schedule next`, `schedule claim` or `setup`.
- `activity start|finish --module M --actor core|dispatcher --phase brief|review|decide [--id ID] [--outcome finished|cancelled|failed]` writes activity events with id `a<seq>`; a finish with nothing open appends nothing, and session end and `schedule release` close open activities. A `heartbeat` event every `AXIOM_HEARTBEAT_MS` (default 30000, minimum 100) records `emitted_at` and `latest_activity_at` and never moves `latest_activity_at`.
- `.axiom/setup.json` has `{"version": 1, "steps": [...]}` with step fields `id`, `argv`, `cwd`, `timeout_s` (1..=7200, default 600), `plan_independent`, `cache_dirs`. Built-in steps are `index` and, when Cargo.toml exists, `cargo-fetch`. Use `orch setup start --repo <dir>`, `orch setup status`, `orch setup keep <id>...` or `orch setup cancel [<id>...]`. Cancel kills the process tree and removes the step's scratch output, never cache dirs or repository files; it also runs on stop and session end. Setup never counts as VERIFY or ACCEPT.
- At unit end, scratch removal is recorded in the storage deletion audit log, including its outcome.
- Setup preflights disk space before starting each step; `cargo-fetch` and steps with cache directories use the cache-build estimate, and other steps use the setup-step estimate.
- In `.axiom/setup.json`, the top-level `estimates` object accepts `wish`, `batch`, `unpack` and `cache_build`, and each step can have an `estimate`; a step estimate takes precedence for that step.
- `orch report` writes `<store>/receipts.json` with `T` (wish window), `N` (final verified non-superseded units), `T/N`, mean concurrency (run overlap integrated over the whole window, including idle gaps), `peak` and `cap`; failed and cancelled runs count.
- Per-attempt scratch writable roots: unit runs get workspace-write with `<store>/scratch/<unit>/<attempt>` as an extra writable root, plus the shared `CARGO_TARGET_DIR` for eligible Cargo repositories; Astra stays read-only.
- Workers and VERIFY commands get private scratch (`TMP`, `TEMP`, `TMPDIR`, pytest `--basetemp`) under `<store>/scratch/`, removed afterwards; nothing is created in the repository.
- The scope check records git-ignored and test-scratch paths as `scope-ignored` and changes made while peers ran as `scope-unattributed` (also in `reports/<unit>.scope.json`); only writes outside the unit's own SCOPE are refused.
- Dispatching a running or verified unit again returns `status: "refused"` with `error.kind` `already_running` or `already_verified` and the `owner` (wish, module, unit, attempt); no Codex turn starts.
- A dispatcher is bound to one module by its first `codex_run` or its first brief inside that module's PATHS; briefs, `unit add --track` and `codex_run` for another module are denied.
- A session that reaches its turn limit stops with exit code 3 and a `chronos` `max_turns` event; the goal is `blocked` with reason `max_turns`, and `axiom wish --resume <session>` continues it.
- `"needs": null` in a verdict reads as omitted, and a brief's SCOPE may span lines or bullets; a repeated SCOPE or a note inside it is rejected at registration.
- The council keeps each member's raw reply, repairs a malformed Argon review once, changes only attribution when anonymising, and records `degraded_members` and `member_degraded` events (progress notifications and `council-events.jsonl`).


### Routing and test-first units

A routed store has a ROUTING block with a default line in every module of modules.md. A map routes every module or none (`routing_partial`). Each block contains indented lines:

```
ROUTING
  <default|unit-id> <tier>/<effort> <risk> <reason...>
```

A unit line overrides default. Precedence is same-attempt snapshot, fix record, map unit line, map default line, brief TIER+EFFORT, brief RISK rubric, explicit model. A disagreeing brief is refused (`routing_mismatch`); a lone TIER or EFFORT gives `routing_incomplete`, and no source gives `routing_missing`. In unit mode role is only a label, not a model or effort choice; settings.models.worker is not consulted. Terra is the floor; luna is refused (`model_floor`). Routing is resolved once per attempt, saved in `routing/<unit>.attempt.json`, and reused on resume; `receipts/routing.jsonl` records assignments and reasons.

Astra assigns units in the plan and the architect writes the map. For an unrouted unit the RISK rubric is trivial terra/medium, standard sol/high, state_machine, concurrency, migration, parser, security, privacy, cross_module and prior_failure sol/xhigh, and novel astra/xhigh. Choose stronger when unsure. The plan records the rubric and each assignment; receipts record resolved routing. The first-pass table summarised in Astra's brief groups historical results by model, effort and risk class; rates require at least three units, with at most 40 rows and 4,000 bytes, including a cutoff and truncation note.

`unit add --fixes <old>` climbs one step: terra/any and sol/low or medium → sol/high; sol/high → sol/xhigh → astra/xhigh → astra/max; sol/max → astra/max; astra/high → astra/xhigh. Astra/max is saturated (`escalation_saturated`). A plan exception uses `unit add ... --fixes <old> --routing <tier>/<effort> --reason <text>`; the last two flags require each other and a nonempty reason. Orch writes TIER/EFFORT after GOAL in the fix brief and rejects disagreement.

CONTRACT and TEST_MATRIX are paired worker-visible sections between GOAL and CONTEXT. Each matrix row is:

```
<id> | <behaviour|edge|failure> | <file>::<case> | <assertion> | <command>
```

Test files must be in SCOPE. Both sections are mandatory in routed stores; a present matrix is always enforced, including in legacy stores. For a wish without a map, dispatchers write both in every brief. The worker writes tests first in a separate RED turn in the same attempt; the engine runs each row command and records RED evidence against the observed test-file tree before implementation. Valid RED reports a failing test whose name contains that row's case. Compile errors, missing commands (127/9009 or command-not-found), and bare nonzero exits without a failing test are not RED; stub signatures so assertions fail. Changing matrix test files in the implementation turn fails with `red_tampered`; self-check may strengthen them. VERIFY runs the matrix commands.

After VERIFY and before review, the engine runs self-check (default sol/xhigh). It compares CONTRACT, TEST_MATRIX, tests and diff, may fix only inside SCOPE, and writes evidence kind `selfcheck` and `reports/<unit>.selfcheck.json`. Self-check reports contain `untested`, `vacuous_check` and `contract_mismatch` findings with their fixes. Review-pack limitation: the current `orch::report::review_pack` includes CONTRACT, TEST_MATRIX and RED/matrix results but does not embed the self-check report; findings are persisted in `reports/<unit>.selfcheck.json`, not automatically supplied to the reviewer. Reviewers are fresh and bound to the verified tree. Only blocker and major findings get one quick Claude verification; minor findings do not. Default routed-store reviewer mode is `astra-max`; explicit `reviewer` values (`claude`, `astra-max`, `both`) win.

### Core models, settings and resume

Core stages planning, build, escalation and close default to opus, sonnet, opus and opus. At turn boundaries the engine derives the stage from store state. Configure the `models` block in policy.json, then `<home>/settings/models.json`, then defaults. The block uses `core` keys `planning`, `build`, `escalation`, `close`; `worker` maps tiers to model ids but is not consulted for unit routing; `self_check` and `reviewer` contain `tier` and `effort`. Global settings wrap `models` with `version`.

The engine first switches via a `control_request` with subtype `set_model`; on error, unsupported subtype or a 10-second timeout, it resumes the same session id with `--model` at the next turn boundary. Confirmed turns are logged in `<home>/goals/<session>.turns.jsonl`.

Settings need no restart: slot cap is read on each acquire, Codex command when constructing the bridge (next wish), and models at new turn boundaries. Started attempts retain their routing snapshot. Resume reruns no verified or built unit; an interrupted unit resumes its own attempt, without superseding it.

Codex's base instructions (~3.5k tokens) stay in place because they teach the shell and patch tools.

<details>
<summary><strong>Recorded lean-mode measurements</strong></summary>

Measured with `gpt-5.6-terra` at low effort:

| Brief | Inherited config | Lean |
| --- | --- | --- |
| One-word reply | 19,863 input tokens | 8,457 |
| Write a module and its tests, then run them | 149,414 input tokens, FAIL (detoured into Pstack skills, then sandbox-blocked) | 29,084, PASS |

</details>

### Native graph and memory

Chronos also has a native code and memory graph, separate from the optional external CodeGraph integration below. The bridge tool `graph_explore` requires `project_dir` and accepts `project`, `query`, `node`, `view` (`code`, `memory`, or `mixed`, default `mixed`), `direction` (`out`, `in`, or `both`, default `both`), `hops` (0–2, default 2), `budget`, and `refresh`. The response budget is a UTF-8 byte upper bound, default 2400 and clamped to 256–4096 bytes; memory bundles have a separate 1200-byte cap. The response reports freshness, revision, omissions and truncation when space permits.

In the default unit mode, `memory_record` can also take typed `nodes` and `links` arrays alongside the usual record. Each node has `key`, `kind`, `label`, `summary`, `attrs`, `state`, and optional `entry_id`; each link has `kind`, `to`, and optional `from`. In `mode: "source"`, `source` specifies an `artifact` and `links` with `to`, `quote`, and optional `line`.

`axiom graph refresh [--dir <absolute-dir>] [--json]` synchronizes the code index and available orchestration data. `axiom graph explore <query> [--dir <absolute-dir>] [--project <slug>] [--view code|memory|mixed] [--budget <bytes>]` prints a graph packet as JSON. `axiom dream` maintains graph memory; its options include `--dry-run`, `--project`, `--dir`, `--max-changes`, `--max-seconds`, and `--json`.

Constraints are read first from the project root's `CONSTRAINTS.md`, then from `<project store>/constraints.md`. Duplicate entries are removed; paragraphs and bullets with their indented continuations are kept as whole entries in order, within a default 1024-byte budget. The resulting block is prepended to unit-mode briefs. `<project store>/aliases.csv`, with header `canonical,alias`, maps aliases to canonical names for graph queries.

The `UserPromptSubmit` hook supplies a bounded launch context packet with the goal notice, including constraints and relevant graph context when available. It is injected once, recorded as `<wish store>/launch.json`; later prompts do not inject it again. See the [A/B protocol](graph-ab.md) for measurement status.

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
