# Storage

Axiom records storage in `<home>/storage/ledger.sqlite3`, records cleanup outcomes in `<home>/storage/deletions.jsonl`, and reads policy from `<home>/storage/policy.json`. Entries have a class and an owner: a project and optional wish; system entries use project `_chronos`. A path belongs to its deepest registered ancestor, so nested entries are accounted for separately. Unmeasured entries are counted as unmeasured; aggregate byte totals include only measured entries.

Classes are `worktree`, `build_cache`, `unit_store`, `scratch`, `engine_copy`, `logs`, `evidence`, `research_cache`, and `download`. Their owners identify the project and, where applicable, the wish; shared system data uses the system owner.

## Inspect and clean

- `axiom storage` lists usage; `--measure` measures entries and `--json` emits JSON.
- `axiom storage clean` plans first; add `--yes` to execute the plan. `--pressure` selects pressure cleanup and `--json` emits the plan and outcome.
- `axiom storage worktrees` lists registered worktrees. `axiom storage worktrees --repo <dir>` adopts linked worktrees from that repository before listing. `axiom storage worktrees remove <path>` plans removal; add `--yes` to act. Dirty or active worktrees are not eligible for removal. A clean worktree is merged when its HEAD is an ancestor of the first available reference among `origin/HEAD`, `main`, then `master`, and it has changed from its registered HEAD; for an adopted worktree without a registered HEAD, its HEAD must differ from that reference.

Routine retention removes unused orphan scratch older than ten minutes, eligible unused build caches after `build_cache_days`, and unused engine copies except the current and most recent previous copy. A cache is eligible when its worktree is merged, abandoned or gone; a shared cache with users is retained. Pressure cleanup can reclaim eligible caches without the age wait. Worktrees are not automatically deleted; unit stores, research caches and downloads are retained. Logs and evidence are not automatically deleted: routine cleanup compresses eligible unused files older than `compress_after_days`.

Archives use `<file>.gz`; when both exist, the plain file wins. Archive-aware appenders restore the plain file before appending. Only files in logs and evidence are compressed; `events.jsonl` and `.lock` files are excluded. The ledger lives outside the standard logs and evidence roots, but there is no separate ledger-filename exclusion if a custom registration makes it part of such a root. The defaults are `build_cache_days: 7`, `compress_after_days: 30`, `low_disk_bytes: 5 GiB` and `reserve_bytes: 2 GiB`.

Cargo worktrees of the same eligible repository share one `CARGO_TARGET_DIR` build cache instead of one `target/` per worktree. Cargo's build-directory lock serializes concurrent builds in that target directory: less duplication in exchange for build contention.

## Disk guard

Preflight checks available space against the estimate plus the reserve and reservations held by other runs. Wishes, setup steps, and managed batch, unpack and cache launches use preflight. If a heartbeat finds low disk space, it first frees eligible safe scratch, build caches and engine copies; if space remains low, it pauses the wish with the distinct `disk_space` stop reason. Free space and resume: a successful resume check clears that reason. `axiom launch` returns exit 75 when preflight pauses; free space and retry the launch.

The policy's default estimates are wish 2 GiB, batch 10 GiB, unpack 10 GiB, cache_build 10 GiB and setup_step 512 MiB. `.axiom/setup.json` can specify a top-level `estimates` object with `wish`, `batch`, `unpack` and `cache_build`, and an `estimate` on each setup step. A step estimate takes precedence over the applicable project estimate, then policy defaults apply. `axiom wish --need-space <size>` and `axiom launch --need <size>` override the estimate. Sizes accept bytes or binary-unit suffixes, for example `512M`, `20G` and `1.5T`.

For tests, `AXIOM_FAKE_DISK` is the path to a JSON file such as `{"free_bytes": 10737418240}`; the probe uses that unsigned byte count and a single fake volume.

## Usage and evolve

`axiom usage` and `axiom evolve` read archived (`.gz`) transcripts, logs, goals and handoffs in place, without restoring or changing them. When a plain file and its archive both exist, the plain file is read and counted once. An unreadable archive is skipped and does not hide other sessions.
