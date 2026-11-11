# Chronos desktop app: interaction design

The app is `chronos` (`axiom app` also opens it). It is a native window built on gpui and gpui-kit. It lets a person make a wish, watch it, steer it and read what happened without a terminal.

Screens draw `desktop::model::AppState` and call its command methods. They never read `~/.axiom` or start processes themselves.

## Principles

- **Say what is true.** A figure that is unknown shows "—", never a guess. A finished model session is not a verified change.
- **One obvious next action.** Every state of a run has a single primary action; the rest stay quiet.
- **The checklist answers "how far along are we?"; the graph answers "what is happening now?"** The checklist is always visible on the Run screen.
- **Calm by default.** Animate only while something is actually running. When nothing runs, the app uses no CPU. `reduce_motion` turns animation off.
- **Never lose work or surprise.** Anything irreversible asks first and says what will happen. Closing the window never silently kills a wish.

## Visual language: the Celestial Order

The source is the Chronos portrait in `assets/chronos.ico`: a blindfolded keeper of time, an hourglass held in orbits, a deco frame with ring and diamond studs, navy, ivory and old gold. The app should feel like that card come to life in a dark observatory. It must not look like a stock SaaS dashboard or a fantasy game.

Mystery comes from low light, empty space and restraint. It never comes from hiding information.

- **One light source.** Gold is the only accent. It marks the current action, the selection and the running path. Everything else recedes into navy until it is needed.
- **Colors.** Use only `theme::Tokens`. Navy night and ivory in dark; a parchment star chart with navy ink in light. State colors are aged pigments (patina, moonlight, amber, cinnabar), never neon. `orbit` is for chart lines and dividers.
- **Shape carries state as well as color.** Verified is a steady star, running a slowly breathing ring, needs review an open eye, failed a broken ring with a cross, waiting a faint point.
- **Frames.** One fixed backdrop with a dim glow, vignette and stars sits behind every screen. The tarot frame (a hairline, a second hairline inset 3px in `orbit`, diamond studs at the corners) is for moments that matter: the needs-you card, empty states, the end-of-run summary, dialogs, and Settings and Usage cards. Ordinary panels are separated by one hairline. Corners stay nearly square, 4px at most.
- **Dividers.** A measured hairline with a small central diamond, as in `assets/chronos-threshold-*.svg`, separates major sections only.
- **Graph.** Keep the tested layout, and draw it as an orrery. The core is the hourglass; modules are ringed bodies; units are stars. Edges are `orbit` hairlines that turn gold only along running work. Behind them sit two or three faint arcs and a sparse fixed star field under 6% opacity, which never move.
- **Type.** Page titles use serif Constantia, with Palatino Linotype as fallback; Chinese titles use bold Microsoft JhengHei. Content is 12px at the very least, with tabular figures for numbers. Section labels use widely spaced gold capitals and the measured divider beneath.
- **Portrait.** It appears small at the top of the rail and large and dim behind empty states. It never sits behind text that has to be read.
- **Motion.** Slow as breath: 1.5 to 3 seconds per cycle, no bounce. Three ceremonies exist, each playing once and then asking for no more frames. In the graph, the orbit fades in when a wish started from this app appears, and the hourglass turns once when a run the app saw unfinished ends (about 1.6 seconds each; runs already on screen when the app opens get neither). In the New wish dialog, the advanced research emblem ignites when the switch is turned on. All three are skipped under `reduce_motion`, and nothing moves while nothing runs.

## Humane use

The myth is in the atmosphere. The facts are in plain words.

**Voice.** Mythic words are allowed only in headings, empty states and ceremonies. Anything stating a fact, error, cost or consequence is plain, and every button says what it does.

| Moment | Text (zh-TW) |
|---|---|
| No runs yet | 星圖尚空。說出你的願望，Chronos 會拆解、動工，並逐項驗證。 |
| Primary button | 許願 |
| Planning | Astra 正在推演計畫… |
| Blocked heading | 等你定奪 |
| Waiting on a batch | 批次仍在執行，等待期間不消耗模型額度 |
| Finished, everything verified and the full test passed | 願望已實現 |
| Finished otherwise | 已結束：3 個單元尚未驗證 |

"願望已實現" is earned: it never appears on a run that was not fully verified.

**Always answer three questions at the top of the Run screen, in one sentence:** what is happening, whether the person needs to act, and what happens next. For example "正在建造 2 個單元，暫時不需要你動手".

**Respect the person's attention.**
- Interrupt only when they are needed. Routine progress never notifies.
- A needs-you card says what happened, what it means, and what Chronos recommends, with that recommendation as the one primary button.
- Do not make people learn the machine. Internal ids, session ids and file paths stay under a "Technical details" fold; role names such as Astra or Critic explain themselves on hover.

**Respect the person's work.**
- A click answers within 100 ms: the control shows that it is working, even if the result takes longer.
- Selection, scroll, filters and expanded rows survive refreshes. Nothing jumps while someone is reading; new content waits or arrives below.
- Text being typed is kept as a draft if the dialog closes or the app quits.
- Confirm only what cannot be undone, and name the action on the button: "停止並關閉", never "確定".
- Errors say what happened, what it affects and what to do next, with the raw message behind Copy. Never a bare code.

**Respect the body.** Targets are at least 28px. Text meets WCAG AA contrast in both themes, which the theme tests enforce. No flashing, and low brightness stays the default in the dark theme.

## Language

The interface is Traditional Chinese by default and English when chosen in Settings or when the system language is English. All text goes through `i18n::tr`. Wish text, file contents and log lines stay as written.

Role names read the same on every screen, from one table in `i18n.rs`: Core (主控), Dispatcher (調度員), Reviewer (審查員), Architect (架構師), Memory keeper (記憶管理員), Critic (第二審) and Astra. The usage charts, the ledger warnings and the source column of the event log all use it, and an id with a plugin prefix such as `chronos:dispatcher` reads as its role. An id that names no role, such as a tool or unit name, is shown as reported. The stage rail names stages by what they do (for example 切模組 for the Architect stage); that is a different thing from the role.

## Keyboard

Everything the keyboard does is listed in one table, `SHORTCUTS` in `palette.rs`. The window's global bindings are made from it, the shortcuts overlay reads it, and a test fails when a screen handles a key that is missing from it.

- **Ctrl+K** opens the command palette (Cmd+K on macOS). It searches wishes, goals, modules, units and memory, and also commands: make a new wish, go to a screen, switch theme or language, and stop or resume the current wish. Matching is fuzzy and highlights the matched letters; a large index is searched in the background so typing never waits. Up and Down, Page Up and Page Down move through the results, Enter goes to the selected one and Escape closes. Choosing a result opens that run, goal, module, unit or memory entry.
- **?** opens the shortcuts overlay, which lists every shortcut grouped by where it works: anywhere, the palette, the New wish dialog, the `@` file list, the message box, the Wishes and Memory lists, the checklist and the event log. It does not open while a text box has focus, where `?` is just a character.
- Ctrl+N makes a new wish, and Tab and Shift+Tab move between controls.

## Navigation

A left rail: **Run**, **Wishes**, **Memory**, **Usage**, **Settings**, with the New wish button above them. The Wishes item shows a badge with the number of runs needing the user. The title bar shows the current project and run status.

## First launch and empty states

The app checks the environment on launch, in the background. If `claude` or `codex` is missing or signed out, the Run screen shows a setup card naming what is wrong and how to fix it, and New wish is disabled with the reason beside it. Optional tools such as codegraph never block.

Until the first wish has finished, a short first-day checklist sits on the Run screen: tools found, signed in, a project chosen, first wish made. Each item ticks itself from real state and the list leaves for good once it is complete.

With no runs: an empty state with one line of explanation and a large **Make a wish** button. Every list has an empty state that says what would appear there. Data that cannot be read shows a quiet warning on the affected panel; the rest of the app keeps working.

## Making a wish (Ctrl+N)

A dialog with:
1. **Project folder**: folder picker plus recent projects. Shows the git branch. Warns when the tree has uncommitted changes: "Commit first so this can be undone". A folder that is not a git repository also warns but can proceed.
2. **Wish**: multi-line text, full IME support, pasting allowed.
3. **Creative** toggle, explained in one line. Advanced: maximum turns.
4. **Start**, disabled with a visible reason until a folder and a wish exist. Starting takes you to the Run screen for that run.

Enter inserts a newline here; Ctrl+Enter starts.

**Advanced research.** A card below the wish has a switch, off every time the dialog opens, for a joint discussion by Astra, Fable 5.2 and Argon before the plan. The card says that Fable is capped at $5 and Argon's quota is small. If `claude` or `agy` is not installed it says so and that the council will run with fewer members; the switch still works. The request carries the choice to the wish. Turning the switch on plays an emblem animation, three orbits sweeping in and igniting in gold in about 1.6 seconds, then the emblem holds still. Turning it off is immediate and quiet, and under `reduce_motion` the card simply shows lit.

**Pointing at files with `@`.** Typing `@` in the wish opens a list of the project's files and folders, ranked by what has been typed after it with the matched letters in bold. It is read in the background: from git when the folder is a repository (so `.gitignore` is followed), otherwise by walking the folder and skipping hidden folders, `node_modules`, `__pycache__`, `target` and what the top-level `.gitignore` names. A very large project is cut off and the list says so. Up and Down choose, Enter or Tab inserts the path (quoted if it contains spaces), and Escape closes the list. The list has at most 50 rows, scrolls, and shrinks to stay inside the window; it is drawn over the controls below the wish box. With no folder chosen, or when the files cannot be read, it says why.

## Run screen

Header: the wish (one line, click to expand), project folder (click to open), status pill, elapsed time, cost so far. A run started elsewhere shows a "Started in terminal: view only" badge.

Below it, the **stage rail**: Astra → Critic → Architect → Build → Full test → Memory → Report, each with done, running, waiting or failed and a short note.

Three columns, with resizable splits:

### Checklist (left, always visible)

The goal's todo list, so it is clear what has to be done and what is finished:
- **Goals**: the plan's numbered items, each with the modules that deliver it, and a state derived from them.
- **Modules**: purpose as a sentence, `verified / total`, expandable.
- **Units**: the title from the brief's GOAL, with state icon and attempt count.

Before a plan exists it says what is happening ("Astra is planning…"). Finished items are checked and dimmed, the current ones stand out, and the list scrolls to keep active work visible unless the user has scrolled away. Clicking an item selects it everywhere. A filter offers All, Active, Needs review and Failed.

### Graph (centre)

Chronos as a live mesh: the core on the left, then modules, then their units. Color carries the state, with a legend. Particles flow only along edges with running work. Hovering shows a tooltip and highlights the path; clicking selects and opens details; clicking empty space clears the selection.

The graph can be zoomed and moved. Ctrl and the wheel zoom about the pointer (roughly 10% a notch), the plain wheel pans, and dragging empty space pans. The corner controls zoom out, show the zoom (a click returns to true size), zoom in, and **Fit**. Zoom runs from 10% to 300%, and some of the graph always stays in view. Fit frames everything, centred and never larger than true size; it, the tooltips and keyboard scrolling all leave the strip at the bottom-right where the controls sit free. A tooltip that would cover the controls moves above or beside them. Each run keeps its zoom and pan while the app stays open.

With the graph focused, the arrow keys move a cursor: right from a module to its units, left back to the module and the core, up and down between siblings. The cursor shows as a ring, scrolls into view, and shows the same tooltip as hovering. Enter selects what the cursor is on, Escape clears it and the selection, F fits, and Ctrl with plus, minus or 0 zoom in, zoom out and return to true size. Large modules collapse verified units into "+12 verified" so failures and active work stay visible.

### Details (right)

- Nothing selected: **Overview**. What is running now, any needs-you card, the decisions Chronos made on the user's behalf, recent events.
- Module: purpose, contract, dependencies and units.
- Unit: state word, attempts, then tabs **Brief**, **Report**, **Verify**, **Review**. Each tab has Copy and Open in editor. File references in the text, such as `src/a.rs:42` or `src/a.rs:42:7`, are links: a click opens the file in the editor at that line (in VS Code when `code` is on the PATH, otherwise in the system's default app). Only files inside the project open, and the check is made against the disk at the click, so a link cannot lead out of the project through a symbolic link; a file that is outside or gone is not opened and a notification says why. Text that merely looks like a name, such as `e.g.` or a bare `README.md`, is not made a link. Large files show their beginning and say that they were cut. On a run started here, **Say something about this** opens the interjection box already addressed to the selected goal, module or unit.

### Control bar (bottom)

What it offers depends on the run:

| Run state | Started here | Primary | Other |
|---|---|---|---|
| Running | yes | Send (interjection) | Stop |
| Running | no | none: "Started in terminal: view only" | |
| Needs you (blocked) | either | Resume, with an optional reply | Open folder |
| Waiting on batch | either | Resume now | Shows the batch's progress |
| Done | either | Make another wish | Open folder, View report |

Interjecting: Enter sends, Shift+Enter makes a newline. The message appears in the log immediately. It may add to the work, ask a question or say stop.

Stop asks first: "Stop this wish? It can be resumed later." Stopping moves it to Needs you.

An event log opens from the control bar; it follows new lines until the user scrolls up and can be filtered by level.

### When a run ends

The centre shows a summary: verified and total units, full test result, what needs attention, cost, and a link to the report. Units that are not verified are listed and never described as done.

## Needs you

When a run blocks or a batch finishes, the app shows a card with Chronos's note and the action. It also shows an in-app notification and flashes the taskbar if the window is not focused.

## Wishes

Every run, newest first: status, project, wish, `verified / total`, cost, start time. Filters: All, Running, Needs you, Done. Search covers wish and project. Click opens the run. A row menu offers Resume, Open folder and Copy session id. Runs are never deleted from here.

## Memory

Titled "What Chronos has learned". Filter by project, search, a list and a detail pane. Each entry says which wish it came from and when it was last confirmed, and it can be corrected in place; a correction becomes the entry's new text and is what later briefs see. Stale entries are marked. Deleting asks first and says it cannot be undone.

## Usage

Cost for this run and across runs by model and role; Peak context per agent with the warning threshold drawn; warnings such as one agent's context growing too large or cache rewrites after long waits; a per-run table. It states that dollars are API-price equivalents, which subscription plans do not bill.

## Settings and health

Environment checks with fix hints and a re-check button; versions of the app, plugin, Claude Code and Codex with any mismatch called out; language; theme (System, Light, Dark); reduce motion; data folder.

## Closing the window

If a wish started here is running, ask: **Keep working** (cancel), **Stop and close** (it can be resumed later). No silent kill. Window size and position, last screen, language and theme persist.

## Accessibility and polish

Everything reachable by keyboard with visible focus. Color is never the only signal: state also has an icon or a word. Long text truncates with a tooltip. Layout holds from the 1100×700 minimum upward and on high-DPI screens, in both themes.
