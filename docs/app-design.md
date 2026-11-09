# Chronos desktop app: interaction design

The app is `chronos` (`axiom app` also opens it). It is a native window built on gpui and gpui-kit. It lets a person make a wish, watch it, steer it and read what happened without a terminal.

Screens draw `desktop::model::AppState` and call its command methods. They never read `~/.axiom` or start processes themselves.

## Principles

- **Say what is true.** A figure that is unknown shows "—", never a guess. A finished model session is not a verified change.
- **One obvious next action.** Every state of a run has a single primary action; the rest stay quiet.
- **The checklist answers "how far along are we?"; the graph answers "what is happening now?"** The checklist is always visible on the Run screen.
- **Calm by default.** Animate only while something is actually running. When nothing runs, the app uses no CPU. `reduce_motion` turns animation off.
- **Never lose work or surprise.** Anything irreversible asks first and says what will happen. Closing the window never silently kills a wish.

## Language

The interface is Traditional Chinese by default and English when chosen in Settings or when the system language is English. All text goes through `i18n::tr`. Wish text, file contents and log lines stay as written.

## Navigation

A left rail: **Run**, **Wishes**, **Memory**, **Usage**, **Settings**, with the New wish button above them. The Wishes item shows a badge with the number of runs needing the user. The title bar shows the current project and run status.

## First launch and empty states

The app checks the environment on launch, in the background. If `claude` or `codex` is missing or signed out, the Run screen shows a setup card naming what is wrong and how to fix it, and New wish is disabled with the reason beside it. Optional tools such as codegraph never block.

With no runs: an empty state with one line of explanation and a large **Make a wish** button. Every list has an empty state that says what would appear there. Data that cannot be read shows a quiet warning on the affected panel; the rest of the app keeps working.

## Making a wish (Ctrl+N)

A dialog with:
1. **Project folder**: folder picker plus recent projects. Shows the git branch. Warns when the tree has uncommitted changes: "Commit first so this can be undone". A folder that is not a git repository also warns but can proceed.
2. **Wish**: multi-line text, full IME support, pasting allowed.
3. **Creative** toggle, explained in one line. Advanced: maximum turns.
4. **Start**, disabled with a visible reason until a folder and a wish exist. Starting takes you to the Run screen for that run.

Enter inserts a newline here; Ctrl+Enter starts.

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

Chronos as a live mesh: the core on the left, then modules, then their units. Color carries the state, with a legend. Particles flow only along edges with running work. Hovering shows a tooltip and highlights the path; clicking selects and opens details; clicking empty space clears the selection. Large modules collapse verified units into "+12 verified" so failures and active work stay visible.

### Details (right)

- Nothing selected: **Overview**. What is running now, any needs-you card, the decisions Chronos made on the user's behalf, recent events.
- Module: purpose, contract, dependencies and units.
- Unit: state word, attempts, then tabs **Brief**, **Report**, **Verify**, **Review**. Each tab has Copy and Open in editor. Large files show their beginning and say that they were cut.

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

Filter by project, search, a list and a detail pane. Stale entries are marked. Deleting asks first and says it cannot be undone.

## Usage

Cost for this run and across runs by model and role; Peak context per agent with the warning threshold drawn; warnings such as one agent's context growing too large or cache rewrites after long waits; a per-run table. It states that dollars are API-price equivalents, which subscription plans do not bill.

## Settings and health

Environment checks with fix hints and a re-check button; versions of the app, plugin, Claude Code and Codex with any mismatch called out; language; theme (System, Light, Dark); reduce motion; data folder.

## Closing the window

If a wish started here is running, ask: **Keep working** (cancel), **Stop and close** (it can be resumed later). No silent kill. Window size and position, last screen, language and theme persist.

## Accessibility and polish

Everything reachable by keyboard with visible focus. Color is never the only signal: state also has an icon or a word. Long text truncates with a tooltip. Layout holds from the 1100×700 minimum upward and on high-DPI screens, in both themes.
