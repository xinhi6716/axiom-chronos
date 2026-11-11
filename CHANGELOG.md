# Changelog

## 2.0.0

Chronos 2.0 adds a Windows desktop app, install packages and engine changes that reduce repeated context and stop more kinds of failure. Data in `~/.axiom` from 1.0.0 stays readable.

### Install packages

- Extract `axiom-chronos-2.0.0-windows-x64.zip`, then run `powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1`. It installs `axiom.exe` and `chronos.exe` to `%LOCALAPPDATA%\Programs\Axiom Chronos`, updates the user PATH and adds a Start menu shortcut. `-Prefix`, `-NoPath`, `-NoShortcut` and `-NoPlugin` control those steps. Close both programs before upgrading 1.0.0 in place. `uninstall.ps1` removes the files, PATH entry and shortcut while keeping `~/.axiom` and the Claude plugin.
- Extract `axiom-chronos-2.0.0-linux-x64.tar.gz`, then run `sh ./install.sh`. It installs `axiom` to `~/.local/bin`; use `--prefix DIR` or `--no-plugin`. Add the folder to PATH if needed.
- Both installers add or refresh the `xinhi6716/axiom-chronos` marketplace and install or update `chronos` when `claude` is available, unless opted out. Both print `axiom doctor`; a failed health check does not fail the install. Bare binaries remain attached, and `SHA256SUMS` covers all release payloads.
- Rust alternative: `cargo install --git https://github.com/xinhi6716/axiom-chronos --tag v2.0.0 --locked --bin axiom`; on Windows, `cargo install --git https://github.com/xinhi6716/axiom-chronos --tag v2.0.0 --locked --features app --bin chronos` adds the desktop app.

### What is optional and what costs money

- **Optional:** the desktop app (a separate program, `chronos`), the advanced research council, the agy backup reviewer and the code graph. Chronos runs without all four.
- **Costs money:** the advanced research council. It is off unless you turn it on with `--advanced-research` or the switch in the New wish dialog. It brings Fable into the planning discussion, and Fable is capped at $5 by default. A council member that has spent its cap starts no new request. Argon has a small quota of its own.
- The summary board, Haiku narrator and optional plain-words translation also make model calls and use your services' quota. Usage dollars are API-price equivalents, not subscription charges.

### The desktop app

- `chronos` is a native Windows window for making a wish, watching it, steering it and reading what happened, with no terminal. Builds with the `app` feature also accept `axiom app`.
- Screens: **Run** (a checklist of goals, modules and units, a live graph of the run, and a detail panel with each unit's brief, report, verify output and review), **Wishes**, **Memory**, **Usage** and **Settings**.
- It shows what is true. A figure that is not known shows a dash, and "wish fulfilled" appears only when everything is verified and the full test passed.
- New wish dialog with a project picker, `@` to point at files, and the advanced research switch (off every time the dialog opens). Ctrl+K searches runs and commands, and `?` lists the shortcuts.
- You can correct or delete a memory entry, say something to a running wish, and follow file references straight into your editor.
- A remaining-quota strip shows Claude and Codex, with an hourglass for each quota window. Missing or stale readings are labelled.
- A mini window can stay always on top. Focus mode and comfortable or dense layouts let you adjust the full window.
- Right-click menus, undo, tooltips and keyboard access cover the controls; the `?` overlay lists the keys.
- A summary board uses Codex luna, falling back to Claude Haiku; you can select Haiku or turn summaries off. Facts, counts and states come from recorded data. A Haiku narrator writes the final report from the engine digest and core rulings; an optional plain-words translation explains technical text.
- The Auto-evolve button previews recent problems and starts a self-improvement pass only after confirmation.
- Closing the window shuts the engine down cleanly. The app reads the same `~/.axiom` records as the command line, so a wish started in a terminal shows up in it, view only.
- Interface in Traditional Chinese or English. Windows builds are attached to the release as `chronos.exe`.

### Cutting token use

- **One dispatcher per module.** Each module gets its own dispatcher, so no single session carries the whole project.
- **Handoff at 150k.** A soft warning appears at 80% of the handoff limit. At 150,000 tokens, a dispatcher hands off to a fresh one, keeping run state on disk. A module that hands off repeatedly with no unit progress is stopped instead of looping. `AXIOM_HANDOFF_TOKENS` changes the limit, and `0` turns handoffs off.
- **Report digest.** The core reads a bounded digest of the run instead of every unit report.
- **Review packs.** The reviewer gets one pack per unit with its contract and diff, instead of searching the tree.
- **Hot memory.** Memory for a brief is a bounded digest of what matters, with deterministic checks for stale entries, duplicates and quotes that no longer match the file.
- **Cache TTL.** Claude sessions ask for the one-hour prompt cache, so waiting between units does not throw the cache away.
- A long batch of your own programs is waited on without a model (`axiom wait`), and Codex reviews stay on the diff, with a raisable token cap.
- Rarely used prompt sections, including the batch and research-council instructions, load only when needed.

### Reliability and safety

- **Races and locks.** The store lock waits its turn and takes over a dead holder's lock one process at a time, even if the process that was cleaning up also died. Goal updates, concurrent memory appends and inbox drains are serialised.
- **Symlink guard.** Opening a file from the app, memory quotes and other path checks refuse links that lead out of the project.
- **Verdicts bound to the reviewed tree.** A review counts only for the files it saw, and a verified unit goes stale when those files change. A stale unit state or a running attempt can no longer pass the done gate, and a CI failure counts as a failed full test.
- **Review carry.** Review results carry forward across sibling fixes when the evidence permits it; `AXIOM_REVIEW_CARRY=0` turns this off. A fix chain stops after two rounds so the core can rule on findings with `orch waive` and a reason, or block the wish.
- **Reviewer-owned verdicts.** Reviewers write their own verdict files with a blocker/major/minor severity rubric. Reports list `needs: smoke/manual` findings for checks a library test cannot reach.
- **Compaction and notes.** Binding rules and the module contract are restored verbatim after compaction. Handoff notes are checked for instructions to hide problems; unit states are derived from evidence, not notes.
- **No-progress stop.** `AXIOM_IDLE_CONTINUES` defaults to 3. A wish with that many unchanged continuations stops instead of being re-prompted.
- **Bounded child processes.** Output captured from child processes is size-limited and time-limited, and the whole process tree is killed, including descendants that hold the output open. Child console windows stay hidden on Windows.
- **Mismatch stop.** A wish stops if the plugin and the binary are different versions.
- **Codex check.** Chronos compares the installed Codex CLI with the versions it was tested against, and `axiom doctor` reports the result.
- **Honest finish.** A wish that has finished says so once. A unit that a later unit replaces can be marked superseded only when the later one covers all of its scope.

### New commands and options

- `axiom review <git range>`: a second opinion on a diff from another model family. Codex reviews first. `--diff-only` sends only the diff, and `--json` prints a machine-readable result. `AXIOM_REVIEW_TOKEN_CAP` raises the token cap (default 400,000).
- `axiom lint`: checks a project's memory for quotes that no longer match, stale entries and duplicates.
- `axiom usage`: a per-run ledger of tokens and cost, with `--session <id>` and `--json`. The desktop Usage screen shows per-model token counts and Codex cost at the official API rates recorded in this release. These are API-price equivalents, not subscription charges; unknown prices stay unknown.
- `axiom evolve --dry-run`: a self-improvement digest of Chronos's own logs for the last seven days. Review it, then explicitly run `axiom evolve --repo PATH` against the Chronos source repository. This weekly pass is on demand, not scheduled. The desktop requires confirmation; the CLI start command does not prompt.
- `axiom wait <session-id>`: picks up a wish that is waiting on a batch, for example after Ctrl-C.
- `--advanced-research`: an opt-in joint discussion by Astra, Fable and Argon before the plan. This is the paid option.
- `--resume <session-id> [text]`: continue a stopped wish with a first message. When Claude reports a usage limit, `axiom wish` waits for the reset and resumes by itself.
- Briefs may carry optional `EFFORT` and `TIER` lines, and `orch unit supersede`, `orch verify` and `orch waive` are new.

### The backup reviewer, agy

- `axiom review` can fall back to agy (Gemini) when Codex is missing or has no quota or login left. It never blocks: a missing, slow or broken agy ends in `skipped` and a normal exit.
- agy is optional and Chronos does not install it. The unit gate still uses `chronos:reviewer`.

### Optional code graph

If a `codegraph` program is on `PATH` and the project has a `.codegraph/` folder, runs use it to cut code reading. Nothing is installed or configured for you.

### Known limits

- `autoCompactWindow` is set in the agent definitions, but its effect has not yet been observed in a real run.
- The handoff-note guard has not yet been seen working in a real run. It is a pattern list and can miss paraphrases. Unit states come from evidence, not notes.
- The desktop app is Windows only. Protocol tests use scripted stand-ins; live-model results vary by project.

## 1.0.0

First release after 0.2.0: Astra plans every wish, Claude reviews and Codex builds, real progress during long Codex turns, and stricter gates in the binary. See [docs/releases/1.0.0.md](docs/releases/1.0.0.md).
