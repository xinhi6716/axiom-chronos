# Changelog

## 2.0.0

Chronos 2.0 adds a desktop app and a large round of changes to the engine: it spends fewer tokens, stops more kinds of failure, and has new commands. Data in `~/.axiom` from 1.0.0 stays readable.

### What is optional and what costs money

- **Optional:** the desktop app (a separate program, `chronos`), the advanced research council, the agy backup reviewer and the code graph. Chronos runs without all four.
- **Costs money:** the advanced research council. It is off unless you turn it on with `--advanced-research` or the switch in the New wish dialog. It brings Fable into the planning discussion, and Fable is capped at $5 by default. A council member that has spent its cap starts no new request. Argon has a small quota of its own.
- A normal wish costs what it did before, or less. Nothing under "Cutting token use" adds to the bill.

### The desktop app

- `chronos` is a native window for making a wish, watching it, steering it and reading what happened, with no terminal. `axiom app` opens the same window.
- Screens: **Run** (a checklist of goals, modules and units, a live graph of the run, and a detail panel with each unit's brief, report, verify output and review), **Wishes**, **Memory**, **Usage** and **Settings**.
- It shows what is true. A figure that is not known shows a dash, and "wish fulfilled" appears only when everything is verified and the full test passed.
- New wish dialog with a project picker, `@` to point at files, and the advanced research switch (off every time the dialog opens). Ctrl+K searches runs and commands, and `?` lists the shortcuts.
- You can correct or delete a memory entry, say something to a running wish, and follow file references straight into your editor.
- Closing the window shuts the engine down cleanly. The app reads the same `~/.axiom` records as the command line, so a wish started in a terminal shows up in it, view only.
- Interface in Traditional Chinese or English. Windows builds are attached to the release as `chronos.exe`.

### Cutting token use

- **One dispatcher per module.** Each module gets its own dispatcher, so no single session carries the whole project.
- **Handoff at 150k.** A dispatcher whose context reaches 150,000 tokens hands off to a fresh one before Claude Code compacts the window. Run state stays on disk, so nothing is lost. A module that hands off repeatedly with no unit progress is stopped instead of looping. `AXIOM_HANDOFF_TOKENS` changes the limit, and `0` turns handoffs off.
- **Report digest.** The core reads a bounded digest of the run instead of every unit report.
- **Review packs.** The reviewer gets one pack per unit with its contract and diff, instead of searching the tree.
- **Hot memory.** Memory for a brief is a bounded digest of what matters, with deterministic checks for stale entries, duplicates and quotes that no longer match the file.
- **Cache TTL.** Claude sessions ask for the one-hour prompt cache, so waiting between units does not throw the cache away.
- A long batch of your own programs is waited on without a model (`axiom wait`), and Codex reviews stay on the diff, with a raisable token cap.

### Reliability and safety

- **Races and locks.** The store lock waits its turn and takes over a dead holder's lock one process at a time, even if the process that was cleaning up also died. Goal updates, concurrent memory appends and inbox drains are serialised.
- **Symlink guard.** Opening a file from the app, memory quotes and other path checks refuse links that lead out of the project.
- **Verdicts bound to the reviewed tree.** A review counts only for the files it saw, and a verified unit goes stale when those files change. A stale unit state or a running attempt can no longer pass the done gate, and a CI failure counts as a failed full test.
- **Bounded child processes.** Output captured from child processes is size-limited and time-limited, and the whole process tree is killed, including descendants that hold the output open. Child console windows stay hidden on Windows.
- **Mismatch stop.** A wish stops if the plugin and the binary are different versions.
- **Codex check.** Chronos compares the installed Codex CLI with the versions it was tested against, and `axiom doctor` reports the result.
- **Honest finish.** A wish that has finished says so once. A unit that a later unit replaces can be marked superseded only when the later one covers all of its scope.

### New commands and options

- `axiom review <git range>`: a second opinion on a diff from another model family. Codex reviews first. `--diff-only` sends only the diff, and `--json` prints a machine-readable result. `AXIOM_REVIEW_TOKEN_CAP` raises the token cap (default 400,000).
- `axiom lint`: checks a project's memory for quotes that no longer match, stale entries and duplicates.
- `axiom usage`: a per-run ledger of tokens and cost, with `--session <id>` and `--json`. A price that is not known shows as unknown, not as a guess.
- `axiom wait <session-id>`: picks up a wish that is waiting on a batch, for example after Ctrl-C.
- `--advanced-research`: an opt-in joint discussion by Astra, Fable and Argon before the plan. This is the paid option.
- `--resume <session-id> [text]`: continue a stopped wish with a first message. When Claude reports a usage limit, `axiom wish` waits for the reset and resumes by itself.
- Briefs may carry optional `EFFORT` and `TIER` lines, and `orch unit supersede` and `orch verify` are new.

### The backup reviewer, agy

- `axiom review` can fall back to agy (Gemini) when Codex is missing or has no quota or login left. It never blocks: a missing, slow or broken agy ends in `skipped` and a normal exit.
- agy is optional and Chronos does not install it. The unit gate still uses `chronos:reviewer`.

### Optional code graph

If a `codegraph` program is on `PATH` and the project has a `.codegraph/` folder, runs use it to cut code reading. Nothing is installed or configured for you.

## 1.0.0

First release after 0.2.0: Astra plans every wish, Claude reviews and Codex builds, real progress during long Codex turns, and stricter gates in the binary. See [docs/releases/1.0.0.md](docs/releases/1.0.0.md).
