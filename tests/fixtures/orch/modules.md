# Module map: chronos-rust (carry out astra/plan-final.md)

Global rules for every module:
- One crate. Cargo package `axiom-chronos`, version `0.2.0-beta.1`, edition 2021, library crate name `axiom`, binary `axiom`. Only the dependencies the `base` module puts in Cargo.toml; add no crate.
- Build and test offline: `cargo test --locked --offline` (CARGO_HOME is E:/codex/.toolchains/cargo; the dispatcher runs `cargo fetch` outside the sandbox first). CI uses `cargo test --locked`.
- Each module lives in its own `src/<module>/` directory (`mod.rs` plus any files it wants) and its acceptance test `tests/<module>.rs`. `src/lib.rs` already declares every module; never edit another module's directory.
- Public APIs take explicit directories and commands (a `home: &Path`, a store path, a resolved command `Vec<String>`). Only `bridge::Config::from_env`, `hook::main_stdin`, `session::Config::from_env` and `cli` read environment variables. Tests never touch the real ~/.axiom: they pass temp directories, or run the binary with AXIOM_HOME/AXIOM_DATA_HOME set to one.
- Errors are `base::Failure { kind, message }` serialized as `{"kind","message"}`; user input never panics. JSON shapes keep the Python key names unless this map says otherwise. Every tool result has `status`; anything other than `completed` carries `error`.
- Text files are read as UTF-8 and tolerate CRLF; files are written with LF. Non-ASCII stays unescaped in JSON written to disk.
- Delete or narrow; add no feature, flag, state or file this map does not name. No comments that restate code.
- Tests use the scripted Python fakes in tests/fakes/ through `tests/common` (Python is a test-only dependency). They must pass on Windows and Linux.

## base
PURPOSE    The crate skeleton and the primitives every other module shares: failures, homes, command resolution, slugs, atomic writes, timestamps, process trees, shell runs, a JSON Schema subset and model families.
PATHS      Cargo.toml
           Cargo.lock
           .gitignore
           src/main.rs
           src/lib.rs
           src/base/
           tests/common/
           tests/base.rs
DEPENDS    none
CONTRACT
  Cargo.toml: [package] name "axiom-chronos", version "0.2.0-beta.1", edition "2021"; [lib] name "axiom" path "src/lib.rs"; [[bin]] name "axiom" path "src/main.rs".
    [dependencies] serde = { version = "1", features = ["derive"] }, serde_json = "1", rusqlite = { version = "0.32", features = ["bundled"] }, sha2 = "0.10", regex = "1"
    [target.'cfg(unix)'.dependencies] libc = "0.2"
    [target.'cfg(windows)'.dependencies] windows-sys = { version = "0.59", features = ["Win32_Foundation", "Win32_System_Threading", "Win32_System_Console"] }
    [dev-dependencies] tempfile = "3"
    Cargo.lock generated offline (`cargo generate-lockfile --offline`) and committed.
  .gitignore keeps its lines and adds `/target/`.
  src/lib.rs declares exactly: `pub mod base; pub mod orch; pub mod codex; pub mod agy; pub mod linux; pub mod memory; pub mod goal; pub mod hook; pub mod bridge; pub mod engine; pub mod session; pub mod cli;`
  src/main.rs: `fn main() { std::process::exit(axiom::cli::main(std::env::args_os().collect())) }`
  Placeholders: base creates `src/<m>/mod.rs` for every other module containing only `//! Owned by module <m>.`, except `src/cli/mod.rs`, which is
    `pub fn main(args: Vec<std::ffi::OsString>) -> i32` printing `axiom-chronos <CARGO_PKG_VERSION> (beta)` for `--version` and returning 0, else printing usage to stderr and returning 2.
    From then on each placeholder belongs to its module, which replaces it.

  pub mod base (src/base/):
  pub struct Failure { pub kind: String, pub message: String }  // Serialize, Debug, Clone, Display "kind: message"
    impl Failure { pub fn new(kind: &str, message: impl Into<String>) -> Failure; pub fn to_json(&self) -> serde_json::Value }
  pub type Res<T> = Result<T, Failure>;
  pub fn resolve_command(env_name: &str, executable: &str) -> Res<Vec<String>>
    A set, non-empty env var must be a JSON array of non-empty-list strings (else kind "config"); otherwise the executable found on PATH
    (Windows also tries PATHEXT extensions), else kind "not_installed" with message "<exe> is not on PATH; install it or set <ENV>".
    An env var set to "" counts as unset.
  pub fn axiom_home() -> PathBuf          // AXIOM_HOME, else <USERPROFILE or HOME>/.axiom
  pub fn data_home() -> Res<PathBuf>      // AXIOM_DATA_HOME, else CLAUDE_PLUGIN_DATA, else Failure "config"
  pub fn require_slug(name: &str, value: &str) -> Res<()>   // ^[a-z0-9][a-z0-9_-]{0,63}$, else "invalid_argument"
  pub fn slug_for_dir(dir: &Path) -> String
    Lowercased final path component; every char outside [a-z0-9_-] becomes '-'; runs of '-' collapse; leading '-'/'_' and trailing '-' trimmed;
    truncated to 64; empty result becomes "project". Always passes require_slug.
  pub fn require_directory(path: &str) -> Res<PathBuf>  // absolute existing directory, else "invalid_argument"
  pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()>
    Writes `.<name>.<pid>.<unique>.tmp` in the same directory, renames over `path`; the temp file never survives an error.
  pub fn now_iso() -> String    // UTC like JS toISOString: "2026-10-30T01:02:03.456Z"
  pub fn today() -> String      // UTC "YYYY-MM-DD"
  pub fn unique() -> String     // 32 lowercase hex chars, distinct across calls and processes (time, pid, counter)
  pub mod proc:
    pub fn command(argv: &[String]) -> std::process::Command   // unix: own process group (process_group(0)); windows: plain
    pub fn kill_tree(child: &mut std::process::Child)
      No-op if exited. Windows: `taskkill /T /F /PID <pid>`. Unix: SIGKILL to the whole process group (-pgid), so grandchildren die too
      (fixes the Python bug that killed only the direct child). Then waits up to 10 s.
    pub fn pid_alive(pid: u32) -> bool   // unix kill(pid, 0) != ESRCH; windows OpenProcess + GetExitCodeProcess == STILL_ACTIVE
    pub struct ShellRun { pub exit_code: i32, pub output: String, pub timed_out: bool }  // output = stdout then stderr, UTF-8 lossy, NULs removed
    pub fn run_shell(command: &str, cwd: &Path, timeout: Duration) -> Res<ShellRun>
      unix `sh -c <command>`, windows `cmd /D /S /C <command>`, stdin null, via proc::command; on timeout kill_tree, exit_code -1, timed_out true.
  pub mod schema:
    pub fn validate(schema: &Value, value: &Value) -> Result<(), String>
      Supports type (object, array, string, integer, number, boolean, null), required, properties, additionalProperties: false, items, enum.
      Unknown keywords are ignored. The error names the JSON path, e.g. "$.changes[0].area: not in enum".
  pub enum Family { OpenAi, Google, Anthropic, Unknown }   // Debug, PartialEq, Clone, Copy
    pub fn family_of(model: &str) -> Family
      "gpt-*", "codex-*", "o<digit>*" -> OpenAi; "gemini-*" -> Google; "claude-*", "opus", "sonnet", "haiku", "chronos:*" -> Anthropic; else Unknown.

  tests/common/mod.rs (`#![allow(dead_code)]`, included with `mod common;`):
    pub fn repo_root() -> PathBuf                 // env!("CARGO_MANIFEST_DIR")
    pub fn python() -> String                     // AXIOM_TEST_PYTHON, else the first of ("python","python3") on Windows / ("python3","python") elsewhere whose --version exits 0
    pub fn fake(name: &str) -> Vec<String>        // [python(), "-I", <repo>/tests/fakes/<name>.py]
    pub fn temp() -> tempfile::TempDir
  Acceptance cases (tests/base.rs): resolve_command override/invalid JSON (config)/missing (not_installed); slug accepts ml_rram, rejects ML_RRAM, _x, ../x, a/b, "";
    slug_for_dir("Chronos Rust") == "chronos-rust" and "TaiwanStockRadar" -> "taiwanstockradar"; atomic_write leaves no .tmp file; now_iso shape;
    kill_tree on a Python child that spawns a sleeping grandchild (pid written to a file) leaves neither alive; run_shell exit codes, CJK output and timeout;
    schema.validate (inline copies of the audit and review schemas, not files that later modules move) accepts a valid sample and rejects a wrong enum, a missing required key and an extra key under additionalProperties false;
    family_of for gpt-6.1-terra, gemini-4-argon-high, chronos:reviewer, sonnet.
ACCEPT     tests/base.rs, `cargo test --locked --offline --test base`

## orch
PURPOSE    The orch store, ported byte-compatible from Pstack's orch.ts, plus brief and module-map parsing and the unit states derived from evidence (registration checks, readiness, evidence, reuse, and the done and Astra gates).
PATHS      src/orch/
           tests/orch.rs
           tests/fixtures/orch/
DEPENDS    base
CONTRACT
  Store format (unchanged from store.ts): units.tsv header `id	track	state	branch	pr	sha	brief` (7 cells), ledger.tsv header `pr	sha	verdict	evidence	verifier	ts` (6 cells),
    inbox/<ts with : and . as ->-<pid>-<unique>.tsv (one 5-cell line), gates.md, preferences.md (numbered lines), frontier.json ("{}\n" when empty), status.md
    (same Markdown template, `Generated: <now_iso>`, trailing `<!-- orch-summary {json} -->`, same "changed" computation). cleanCell, requiredCell, requiredLine,
    CRLF-tolerant reads, header and row-width checks, verdict enum, gate and standing parsing behave exactly as in store.ts. All writes use base::atomic_write.
    Lock: `.orch.lock` holding the pid, created exclusively; a dead holder (base::proc::pid_alive false) is replaced with stderr
    "replacing stale store lock (pid N is dead)"; --force steals with "stealing store lock held by pid N"; otherwise error "store lock held by pid N".
    Inside one process, calls on the same store are serialized by a process-wide mutex keyed by the store path before the file lock is taken.
  pub fn store_for(home: &Path, project: &str) -> Res<PathBuf>   // require_slug, then <home>/orchestrate/<project>
  pub struct Output { pub exit_code: i32, pub stdout: String, pub stderr: String }
  pub fn run(store: &Path, args: &[String], json: bool, force: bool) -> Output
    Commands and compact/JSON output exactly as orch.ts: init; unit add <id> --track <t> [--brief <path>]; unit set <id> --state <s> [--branch] [--pr] [--sha];
    unit get <id>; unit list [--state] [--track]; unit counts; ledger record <pr> <sha> <verdict> --evidence <p> [--verifier <n>]; ledger check <pr> <sha>;
    ledger summary; inbox push <agent> <unit> <status> [--report <p>]; inbox drain [--peek]; inbox count; gate park <id> --question --options --default;
    gate list; gate resolve <id> --answer; frontier show; status; standing show; standing add <line>. Exit 0 success, 1 user or usage error
    (stderr "error: <message>"), 2 not found (ledger check prints NOT-VERIFIED). `frontier set` is removed: exit 1 "error: frontier set is not supported".
    Changed behaviour:
    - unit add --brief <path>: the file must parse with parse_brief (else exit 1 "error: brief <path> is missing <FIELD>"). When <store>/modules.md exists it must
      parse with parse_map (else exit 1 with the map error), and when --track names a module every SCOPE entry must be covered by that module's PATHS
      (else exit 1 "error: <path> is outside module <id>"). Units are not checked against each other's SCOPE (fix and sol units repeat paths).
    - unit set --state built|verified: exit 1 "error: state <s> is derived from evidence; it cannot be set". Other states are written as given.
    New commands (compact text; with --json the value shown):
    - map: parse <store>/modules.md, print one line per module `id<TAB>depends,..<TAB>paths,..`; exit 1 with the reason when invalid or missing.
    - ready: the unit ids `ready_units` returns, one per line, or "(none ready)".
    - accept <module> --repo <dir>: record_accept; prints `<module>	exit=<n>` and the output tail; exit 0 when the command passed, 1 otherwise.
    - fulltest --repo <dir> [--command <cmd>]: record_full; same printing and exit rule.
    - review <unit>... --reviewer <name> --verdict-file <path>: reads {"verdict","findings"} JSON, the pass/fail rule of record_review; exit 1 on bad JSON or same_family.
    - evidence <unit>: the unit's evidence rows as TSV.
  Briefs and maps:
  pub struct Brief { pub goal: String, pub scope: Vec<String>, pub verify: String, pub context: Option<String>, pub text: String }
  pub fn parse_brief(text: &str) -> Result<Brief, String>
    A field starts at column 0 with GOAL, SCOPE, VERIFY or CONTEXT followed by whitespace or end of line; following lines that start with whitespace continue it.
    GOAL, SCOPE and VERIFY must be non-empty. SCOPE entries are split on whitespace and commas. Text before GOAL is ignored.
  pub struct Module { pub id: String, pub purpose: String, pub paths: Vec<String>, pub depends: Vec<String>, pub contract: String, pub accept_file: String, pub accept_command: String }
  pub struct Map { pub preamble: String, pub modules: Vec<Module>, pub full_test: Option<String> }
  pub fn parse_map(text: &str) -> Result<Map, String>
    Text before the first "## " line is the preamble (global rules). Each "## <id>" section has PURPOSE, PATHS, DEPENDS, CONTRACT, ACCEPT with the same column-0 rule;
    DEPENDS "none" means empty. ACCEPT: accept_file is its first token without a trailing comma; accept_command is the first `backtick` span, else the rest of the line.
    A column-0 line "FULL TEST <value>" ends the last module; full_test is its first backtick span, else the value.
    Rejects (Err names the module): a missing or empty field, a duplicate id, a DEPENDS on an unknown or later-declared module, a dependency cycle,
    and two modules whose PATHS overlap.
  pub fn covers(patterns: &[String], path: &str) -> bool   // '\' -> '/', leading "./" stripped; exact match, or the pattern ends with '/' and is a prefix
  pub fn overlaps(a: &[String], b: &[String]) -> bool      // some entry of one covers or equals an entry of the other
  Evidence and derived states:
  pub struct Unit { pub id: String, pub track: String, pub state: String, pub branch: String, pub pr: String, pub sha: String, pub brief: String }
  pub fn read_units(store: &Path) -> Res<Vec<Unit>>
  pub fn unit_brief(store: &Path, unit: &str) -> Res<Brief>      // from the brief path registered in units.tsv; "not_found" / "no_brief"
  pub struct Evidence { pub kind: String, pub command: String, pub exit_code: i32, pub commit: String, pub ts: String }
    Stored in <store>/reports/<unit>.evidence.tsv, header `kind	command	exit_code	commit	ts`, appended under the store lock.
    Kinds: run (command = model id; 0 completed, 1 otherwise), scope (command = offending paths space-joined; 0 none), verify, accept, review (command = reviewer; 0 pass, 1 fail).
    Store-wide rows: <store>/reports/_full.evidence.tsv (kind full) and <store>/reports/_stages.evidence.tsv (kinds astra, second_review, rewrite; command = model).
  pub fn evidence(store: &Path, unit: &str) -> Res<Vec<Evidence>>
  pub fn snapshot(repo: &Path, scope: &[String]) -> String
    The git tree id of the SCOPE paths as they are in the working tree, built with a temporary GIT_INDEX_FILE (`git add -A -- <paths>` then `git write-tree`);
    "" when repo is not a git work tree. Same content gives the same id.
  pub fn derive_state(rows: &[Evidence], needs_accept: bool) -> String
    Only rows after the last `run` row count. No run row: "pending". Run failed, or scope exit != 0, or the latest verify exit != 0: "failed".
    Verify passed and needs_accept and the latest accept row after it exits != 0: "accept-failed"; with no accept row yet: "built".
    Then the review rows after the latest verify (and accept): take those whose commit equals the commit of the latest review row; any fail: "review-failed";
    all pass and none from the worker's family (family_of of the run row's model): "verified"; no review rows: "built".
    needs_accept is true when <store>/modules.md exists and the unit's track is a module id.
  Dispatch lifecycle (used by bridge; tests drive it with a fake worker closure):
  pub enum Begin { Run { brief: Brief, attempt: u32 }, Reused }
  pub fn begin_unit(store: &Path, unit: &str, role: &str, repo: &Path, astra_required: bool) -> Res<Begin>
    Reused when the unit's derived state is verified and snapshot(repo, scope) equals the commit of its latest verify row (no worker, test or review reruns).
    Errors: not_found, no_brief; not_ready (a unit of a DEPENDS module is not verified; lists them); busy (SCOPE overlaps a unit currently running in this process);
    fix_limit (the unit already has 2 run rows); sol_not_allowed (role "sol" while the latest _full row is missing or exit 0); astra_gate (astra_required and astra_gate fails).
    On Run, claims the unit in the process-wide running set and remembers the git dirty set (`git status --porcelain -z --untracked-files=all`).
  pub struct Finished { pub state: String, pub verify_exit: Option<i32>, pub verify_tail: String, pub outside_scope: Vec<String> }
  pub fn finish_unit(store: &Path, unit: &str, repo: &Path, role: &str, model: &str, report: &Value) -> Res<Finished>
    Always releases the claim. Appends the run row; writes report.final_message (or "") to reports/<unit>.md and the whole report to reports/<unit>.json;
    `inbox push <role> <unit> <report status> --report <md path>`. Outside-scope paths = report.file_changes paths (relative to repo) plus paths newly dirty since begin,
    minus the unit's SCOPE and the SCOPEs of units still running; appends a scope row. If the run completed and nothing is outside scope, runs VERIFY with
    base::proc::run_shell in repo (timeout 1800 s) and appends a verify row whose commit is snapshot(repo, scope); verify_tail is the last 2000 chars.
    Writes the derived state to units.tsv and returns it.
  pub fn abort_unit(unit: &str)                       // releases the claim without rows
  pub fn running() -> Vec<String>                     // currently claimed unit ids
  pub fn ready_units(store: &Path) -> Res<Vec<String>> // pending or failed, fewer than 2 runs, DEPENDS satisfied, no SCOPE overlap with each other's earlier entry in the list or with running units
  pub fn record_accept(store: &Path, module: &str, repo: &Path) -> Res<ShellRun>   // runs the module's ACCEPT command, appends an accept row to every unit of that track, rederives their states
  pub fn record_full(store: &Path, repo: &Path, command: Option<&str>) -> Res<ShellRun> // command or the map's FULL TEST (else invalid_argument); appends a _full row with commit = HEAD sha or ""
  pub fn record_review(store: &Path, units: &[String], reviewer: &str, pass: bool) -> Res<Vec<(String, String)>>
    Rejects with "same_family" when family_of(reviewer) equals the family of any run row of those units. Appends a review row (commit = snapshot of that unit's SCOPE)
    to each unit, rederives and returns (unit, state) pairs. The caller decides pass: false whenever any finding is blocker or major.
  pub fn record_stage(store: &Path, kind: &str, model: &str, ok: bool) -> Res<()>
  pub fn astra_gate(store: &Path) -> Res<()>
    Requires an `astra` stage row with exit 0 and a model for which family_of is OpenAi and the id ends with "-astra", a later second_review row (any exit),
    and the files astra/plan.json, astra/review.json, astra/ruling.md, astra/plan-final.md. Error kind "astra_gate" names what is missing.
  pub fn done_gate(store: &Path, change_mode: bool) -> Res<()>
    If units.tsv has units: every unit verified (error "not_done" lists id=state of the others), the latest _full row exit 0 with ts after every unit evidence row,
    and a rewrite stage row (any exit) after every unit evidence row. No store or no units: Ok unless change_mode, then "not_done" ("no verified units").
  Acceptance cases (tests/orch.rs): round trip of every ported command against expected compact and --json output; tests/fixtures/orch/ old-format stores
    (CRLF rows, empty cells, existing states such as review-failed, an inbox pointer and a resolved gate copied from a real store) read and rewritten unchanged;
    stale-lock replacement and --force; brief missing VERIFY rejected; SCOPE outside the track's module rejected; map with cycle, unknown dep, overlapping PATHS
    and missing ACCEPT rejected; this very map (C:/Users/gppup/.axiom/orchestrate/chronos-rust/modules.md copied into tests/fixtures/orch/modules.md) parses;
    unit set verified and built refused; derive_state table (pending, failed, built, accept-failed, review-failed, verified, same-family review not verified,
    newer review commit supersedes an older fail); with a fake worker closure in a temp git repo: two independent units run overlapping in time on two threads,
    a dependent unit gets not_ready until its dependency is verified, overlapping SCOPEs get busy, a write outside SCOPE ends failed, a third dispatch gets fix_limit
    even after reopening the store, a verified unit with unchanged files returns Reused and the worker closure is not called, and after editing one of its files
    it runs again; sol_not_allowed before a failing fulltest; done_gate refuses an unfinished unit and a stale full test.
ACCEPT     tests/orch.rs, `cargo test --locked --offline --test orch`

## codex
PURPOSE    Drive one long-lived `codex app-server` over stdio JSON-RPC: resolve role models, run ephemeral lean turns, and return typed reports that never claim success they did not get.
PATHS      src/codex/
           tests/codex.rs
           tests/fakes/fake_codex.py
DEPENDS    base
CONTRACT
  pub const SANDBOXES: [&str; 2] = ["read-only", "workspace-write"];
  pub struct RunRequest { pub brief: String, pub cwd: String, pub model: String, pub sandbox: String, pub effort: Option<String>,
    pub output_schema: Option<Value>, pub developer_instructions: Option<String>, pub timeout_s: u64, pub lean: bool,
    pub keep_plugins: Vec<String>, pub web_search: bool }
  pub struct AppServer   // Send + Sync; one child process shared by concurrent turns
  impl AppServer {
    pub fn new(command: Vec<String>) -> AppServer            // starts lazily: `<command> app-server --listen stdio://`, initialize {clientInfo}, then `initialized`
    pub fn alive(&self) -> bool
    pub fn close(&self)                                       // base::proc::kill_tree
    pub fn list_models(&self) -> Res<Vec<String>>             // model/list, following result.nextCursor (sent back as params.cursor) until absent; id from "id", else "model"
    pub fn resolve_model(&self, role: &str) -> Res<String>
      Candidates match ^gpt-(\d+(?:\.\d+)*)-<role>$ exactly (gpt-6-terra-lite and gpt-6-terran do not match terra). Highest version by numeric segments
      (6.10 > 6.9 > 6 > 5.6). None: list again once, then Failure "model_not_found". Never substitutes another role.
    pub fn run(&self, req: &RunRequest) -> Res<Value>
      Argument errors are Failure "invalid_argument" (cwd via base::require_directory, sandbox not in SANDBOXES, empty brief, empty model).
      Otherwise a report with the Python keys: status, thread_id, turn_id, model, final_message, structured_output, commands [{command,status,exit_code}],
      file_changes [{path,kind,status}], refused_requests, usage (tokenUsage.total), disabled, started_mcp_servers, error.
      thread/start {model, ephemeral: true, sandbox, approvalPolicy: "never", cwd, developerInstructions?, config? (lean)}; turn/start {threadId, input, effort?, outputSchema?, disabledPluginIds?}.
      Lean config and disabled lists exactly as codex_app.py (plugin/installed, config/read, features apps/goals off, skills max_context_tokens 1, personality none,
      web_search "disabled" unless req.web_search). Server requests are refused with -32601 and listed in refused_requests.
      Status: the turn's status (completed, failed, interrupted), "timeout" (turn/interrupt sent), or "bridge_error" with kind codex_exited (the next run restarts the server).
      Errors map codexErrorInfo as codex_app.py does, including provider codes such as model_not_found inside "other".
      With output_schema: a completed turn whose final message is missing, is not JSON, or fails base::schema::validate becomes status "failed" with kind
      "invalid_structured_output" (message says which); only a valid one sets structured_output and clears final_message.
      error.user_action = true when the error kind contains "usageLimit" or "auth" (case-insensitive).
    pub fn cancel_all(&self)       // turn/interrupt for every in-flight turn; their reports end "interrupted"
  }
  pub fn role_matches(role: &str, model: &str) -> bool   // the resolve_model pattern
  tests/fakes/fake_codex.py keeps its JSONL protocol and scenarios and adds: model/list paginated over two pages (cursor), with ids including gpt-6-luna,
    gpt-5.6-terra, gpt-6.9-terra, gpt-6.10-terra, gpt-6-terra-lite, gpt-6-astra, in non-sorted order; a brief containing BADJSON makes the final message non-JSON
    under an outputSchema; WRONGSHAPE returns JSON that misses a required key; SLOW waits 2 s before completing (concurrency test).
  Acceptance cases (tests/codex.rs, ports of CodexAppServerTests): completed turn report; lean config and disabled lists; web_search kept; keep_plugins and inherit mode;
    output schema parsed; BADJSON and WRONGSHAPE are failed/invalid_structured_output, never completed; usage limit is failed with user_action; provider rejection
    names model_not_found; approval refused not hung; timeout interrupts; crash reported then restarted; one server for sequential turns; two SLOW turns on two threads
    overlap; resolve_model picks gpt-6.10-terra across pages, errors model_not_found for sol after one refresh (two model/list rounds); invalid arguments rejected.
ACCEPT     tests/codex.rs, `cargo test --locked --offline --test codex`

## agy
PURPOSE    Drive Antigravity (agy) print mode for plain-language rewrites and the cross-family code review, reading success from the result event, never the exit code.
PATHS      src/agy/
           tests/agy.rs
           tests/fakes/fake_agy.py
DEPENDS    base
CONTRACT
  pub fn run(command: &[String], prompt: &str, cwd: Option<&Path>, model: &str, json_schema: Option<&Value>, timeout_s: u64) -> Res<Value>
    Same argv, stdin event, empty-temp-directory rule and result shapes as agy_print.py: status completed | failed (agy_<status>, empty_response) | timeout | bridge_error
    (agy_no_result with the stderr tail). With json_schema, a SUCCESS whose structured_output is missing or fails base::schema::validate is failed/invalid_structured_output.
    The child is registered so cancel_all can kill it; base::proc::kill_tree on timeout.
  pub fn cancel_all()                           // kills every in-flight agy child of this process
  pub const MAX_FILE_BYTES: usize = 60_000; pub const MAX_TOTAL_BYTES: usize = 60_000;
  pub const CHECKS: [&str; 4] = ["contract_mismatch", "vacuous_check", "untested", "platform"];
  pub fn review(project_dir: &str, paths: &[String], contract: &str, model: &str, timeout_s: u64, command: &[String]) -> Res<Value>
    Same instruction text, schema, inline files and path checks as review.py (empty contract, empty paths, a path outside project_dir or missing: Failure invalid_argument).
    Changed: one agy call, no retry on empty_response. A file over MAX_FILE_BYTES or a total over MAX_TOTAL_BYTES returns
    {status: "failed", error: {kind: "too_large", message, fallback: true}} without calling agy. Any agy failure is returned with error.fallback = true.
    Completed: {status: "completed", verdict: "fail" if any blocker/major finding or verdict fail else "pass", findings, reviewer: model, bytes_reviewed, usage, error: null};
    a missing or invalid verdict/findings is failed/no_review_output with fallback true.
  tests/fakes/fake_agy.py keeps its protocol and scenarios; it adds AGY_BADSHAPE (structured_output missing a required key) and counts calls in the file named by FAKE_AGY_COUNT when set.
  Acceptance cases (tests/agy.rs, ports of AgyTests and ReviewTests): empty success failed; no cwd runs in an empty directory; schema success; AGY_ERROR with exit 0 failed;
    missing result explicit; timeout kills the process; AGY_BADSHAPE failed; review derives fail from a blocker despite verdict pass; structured-output case table;
    paths outside the project rejected; oversize returns too_large with fallback and agy is not called; AGY_EMPTY review makes exactly one agy call.
ACCEPT     tests/agy.rs, `cargo test --locked --offline --test agy`

## linux
PURPOSE    Run `python <args>` in the project on Linux through WSL, where "unavailable" is never a pass.
PATHS      src/linux/
           tests/linux.rs
           tests/fakes/fake_wsl.py
DEPENDS    base
CONTRACT
  pub fn run(wsl: Option<Vec<String>>, on_windows: bool, project_dir: &str, args: &[String], timeout_s: u64) -> Res<Value>
    wsl is the resolved AXIOM_WSL_CMD / wsl.exe command or None. Behaviour and shapes exactly as linux.py: invalid_argument for a bad directory or empty args;
    None, or not on_windows without an override, gives {status: "unavailable", reason, error: null}; exit 127 gives unavailable with output_tail;
    otherwise completed or failed (kind linux_command_failed) with exit_code and output_tail (last 4000 chars, NULs removed); timeout is Failure "timeout".
    Argv: <wsl> --cd <dir> -e sh -c 'exec "$HOME/.axiom/linux-venv/bin/python" "$@"' axiom-linux <args...>.
  Acceptance cases (tests/linux.rs, ports of LinuxTests): runs in the project and reports exit codes; FAKE_WSL_NO_VENV is unavailable; no runner is unavailable; empty args rejected.
ACCEPT     tests/linux.rs, `cargo test --locked --offline --test linux`

## memory
PURPOSE    Long-run memory in the existing SQLite FTS5 trigram database, readable and writable in its current format, accepting only verified units.
PATHS      src/memory/
           tests/memory.rs
DEPENDS    base, orch
CONTRACT
  pub fn query_terms(query: &str) -> Vec<String>   // as memory.py: lowercase words [A-Za-z0-9_./-]{2,}, then CJK runs as bigrams (single char kept), order kept, deduplicated
  pub struct Memory   // opens <home>/memory/memory.db per call; the schema statements are memory.py's SCHEMA verbatim (IF NOT EXISTS), so an existing db is never altered
  impl Memory {
    pub fn open(home: &Path) -> Res<Memory>
    pub fn record(&self, args: &Record) -> Res<Value>
      pub struct Record { pub project: String, pub project_dir: String, pub unit: String, pub title: String, pub summary: String, pub body: String,
                          pub symbols: Vec<String>, pub files: Vec<String>, pub wish: String }
      Validation as memory.py, plus: the unit must exist in orch::store_for(home, project) with state verified (else "not_verified"); every file must be covered by
      that unit's own brief SCOPE (else "outside_scope"; a file from another unit's brief is refused); body over 12 lines is "invalid_argument".
      Stores files as [{"path","sha256"}] (hex), symbols space-joined, created = epoch seconds (REAL). Returns {status, id, files, error: null}.
    pub fn search(&self, query: &str, project: Option<&str>, limit: i64) -> Res<Value>   // memory.py's SQL, ranking (matched DESC, created DESC) and shape
    pub fn get(&self, id: i64) -> Res<Value>                                               // entry with files parsed, symbols split, stale
    pub fn digest(&self, project: &str, add: Option<&str>) -> Res<Value>                   // appends "- <base::today()> <whitespace-collapsed line>\n"
  }
  stale: true when a recorded file changed or vanished, false when all match, null when project_dir is gone.
  Acceptance cases (tests/memory.rs, ports of MemoryTests with a verified unit fixture written straight into a temp store): Chinese and English queries; wish text makes
    English entries findable in Chinese; project filter and misses; stale true/false/null; digest append; bad input; unverified unit refused; a file from another
    unit's brief refused; 13-line body refused; old-format compatibility: a db created with the Python SCHEMA and rows inserted by plain SQL (ids 1..3, CJK text)
    is searched and fetched with the same ids, order and stale values, and record appends id 4 without changing sqlite_master.
ACCEPT     tests/memory.rs, `cargo test --locked --offline --test memory`

## goal
PURPOSE    The wish goal of one Claude session as a JSON file that keeps the old format, with its legal transitions: open, interjection, classification, mode, questions, done and blocked.
PATHS      src/goal/
           tests/goal.rs
DEPENDS    base, orch
CONTRACT
  File <home>/goals/<session>.json, session ^[A-Za-z0-9-]{1,64}$ (else invalid_argument). Written with base::atomic_write, JSON indented by 1 space, non-ASCII kept.
  pub struct Goal {
    pub wish: String, pub cwd: Option<String>, pub status: String /* active|done|blocked */, pub amendments: Vec<String>, pub note: Option<String>,
    pub project: Option<String>, pub mode: Option<String> /* review|answer|change */, pub mode_locked: bool, pub asked: bool, pub dispatched: bool,
    pub reason: Option<String> /* "user_stop" */, pub interjection: Option<Interjection>,
    pub other: serde_json::Map<String, Value>   // #[serde(flatten)]: unknown keys survive a rewrite
  }  // new fields are optional with defaults and omitted when unset/false, so files written by the Python version load and old readers still find wish/cwd/status/amendments/note
  pub struct Interjection { pub prompt: String, pub prior_status: String, pub prior_note: Option<String> }
  pub fn load(home: &Path, session: &str) -> Res<Option<Goal>>
  pub fn open(home: &Path, session: &str, wish: &str, cwd: Option<&str>) -> Res<Goal>    // fresh active goal; project = slug_for_dir(cwd) when cwd is given
  pub fn interject(home: &Path, session: &str, prompt: &str) -> Res<Option<Goal>>
    None (no change) when there is no goal or prompt.trim_start() starts with '<'. Otherwise appends the prompt verbatim to amendments, sets
    interjection {prompt, prior_status, prior_note}, status active, mode_locked false (the user's words come first: unclassified means amend).
  pub fn classify(home: &Path, session: &str, kind: &str) -> Res<Goal>    // requires a pending interjection, else invalid_argument
    amend: clears interjection. answer: restores prior_status and prior_note, removes that prompt from amendments, clears interjection (no new work; done/blocked stay closed).
    stop: status blocked, note = the prompt verbatim, reason "user_stop", clears interjection.
  pub fn set_mode(home: &Path, session: &str, mode: &str) -> Res<Goal>    // review|answer|change; refused ("mode_locked") while mode_locked; sets mode_locked true
  pub fn mark_asked(home: &Path, session: &str) -> Res<()>
  pub fn mark_dispatched(home: &Path, session: &str) -> Res<()>
  pub fn set_status(home: &Path, session: &str, status: &str, note: Option<&str>) -> Res<Value>
    status done or blocked only; blocked needs a non-blank note; no goal is not_found.
    done runs orch::done_gate(store_for(home, project), mode == Some("change")) when project is set and refuses with its Failure.
    Returns {"status": "completed", "goal": <goal json>}.
  pub fn find_active(home: &Path, project: &str) -> Res<Option<(String, Goal)>>   // the most recently modified active goal for project
  pub fn astra_requested(goal: &Goal) -> bool    // the wish starts with "[astra]" or "[no-architect] [astra]"
  Acceptance cases (tests/goal.rs, ports of test_goal's state rules): Python-format files (with and without note, CJK wish) load and survive a save with keys kept;
    for prior states active, done and blocked: an interjection reopens to active, then amend keeps active, answer returns to the prior state and note with the
    question removed from amendments, stop closes blocked with the verbatim CJK prompt and reason user_stop; '<task-notification>' leaves the goal untouched;
    classify without an interjection refused; set_mode once, second refused, allowed again after an interjection; blocked without note refused; done refused
    while a unit in the project's store is not verified and accepted once all are verified with a passing fulltest and a rewrite row; atomic save leaves no temp file.
ACCEPT     tests/goal.rs, `cargo test --locked --offline --test goal`

## hook
PURPOSE    Claude Code hook handling for wish sessions: open goals, record interjections, hold the session until the goal is closed, and refuse the tool calls the core and the task mode may not make.
PATHS      src/hook/
           tests/hook.rs
DEPENDS    base, goal
CONTRACT
  pub fn handle(event: &Value, home: Option<&Path>, headless: bool) -> Option<Value>
    home None (no data directory configured) means no goal exists. Never opens SQLite or starts a model; reads and writes only goal files.
    UserPromptSubmit: a prompt matching ^\s*/chronos:wish\b(.*) (dot matches newline) opens the goal and returns additionalContext naming the session id, the program
      slug, "Reply in the language of the wish." and, when headless, "This run is headless: ask the user nothing."; any other prompt goes to goal::interject and, when it
      recorded one, returns additionalContext saying the message amends Chronos goal <id>, that the user's words take precedence over the plan, and to classify it
      with the bridge goal tool as amend, answer or stop. Otherwise None.
    Stop: when the goal is active, {"decision": "block", "reason": stop_reason(..)}; else None.
    PreToolUse (deny = {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": <why>}}; allow = None):
      The core is an event without agent_id; a subagent has agent_id. Only agent_id presence is relied on.
      - mcp__plugin_chronos_bridge__codex_run from the core: always denied, goal or not (as today).
      - The rules below apply only when the session has a goal.
      - Core Write/Edit/NotebookEdit (file_path or notebook_path): denied unless the lexically normalized path is inside home.
      - Core Bash: denied unless read_only_bash(command).
      - AskUserQuestion: denied when headless, from a subagent, when goal.asked, when goal.dispatched, or with more than 4 questions in tool_input.questions;
        otherwise goal::mark_asked and allowed.
      - Mode (any caller): unless goal.mode is "change", Agent/Task with subagent_type "chronos:dispatcher" and codex_run with sandbox other than "read-only" are denied
        (reason: declare the mode with the goal tool when unset; the mode is review/answer otherwise). With mode review or answer, also deny Write/Edit/NotebookEdit
        outside home and Bash that is not read_only_bash, from every caller. With mode change, the core Agent call for chronos:dispatcher is allowed and goal::mark_dispatched.
  pub fn read_only_bash(command: &str) -> bool
    True only when the command has no output redirection other than 2>&1, >/dev/null, 2>/dev/null, no $( or backtick, and every segment split on && || ; | and
    newlines starts (after VAR=value words) with: ls, cat, head, tail, wc, pwd, echo, grep, rg, which, type, find without -delete/-exec/-fprint,
    or git with subcommand status, diff, log, show, rev-parse, ls-files, blame.
  pub fn stop_reason(session: &str, goal: &goal::Goal) -> String   // hooks/goal.py's Stop text, plus a sentence asking to classify a pending interjection
  pub fn main_stdin() -> i32
    Reads all stdin bytes as UTF-8 (lossy), home = base::data_home().ok(), headless = AXIOM_HEADLESS non-empty; prints the reply JSON or nothing; returns 0.
    Unparseable input or an unknown event writes one stderr line and returns 0.
  Acceptance cases (tests/hook.rs, ports of GoalHookTests through handle): wish holds the session until done; later messages reopen with "amends"; a question keeps an
    active goal blocked; task notifications do not reopen; ordinary sessions untouched (no goal: core Write and Bash allowed, core codex_run denied);
    blocked needs a reason; headless context; only agents call codex. Plus: core Write to a project path denied and to <home>/orchestrate/x/astra/ruling.md allowed;
    core `echo x > f.txt`, `rm -rf src`, `git commit -am x`, `cat a; touch b`, `echo $(rm x)` denied and `git status`, `git diff -- src | head -50`, `ls -la` allowed;
    dispatcher (agent_id set) codex_run allowed in change mode; dispatcher Agent and workspace-write codex_run denied before a mode is declared;
    in review mode a subagent Write to the project and `sed -i` Bash denied; AskUserQuestion with 5 questions, a second call, a headless call and a call after
    dispatch denied, a first call with 4 questions allowed; "可以把 X 改成 Y 嗎" and "不要停" are recorded as amendments (never a stop);
    the hook never creates <home>/memory; a timing check runs `handle` 20 times and prints the median (not asserted; the binary's cold start is measured in cli).
ACCEPT     tests/hook.rs, `cargo test --locked --offline --test hook`

## bridge
PURPOSE    The MCP stdio server: the same eleven tools as the Python bridge, each a thin typed call into orch, codex, agy, linux, memory and goal, with isError derived from the result type.
PATHS      src/bridge/
           tests/bridge.rs
DEPENDS    base, orch, codex, agy, linux, memory, goal
CONTRACT
  pub struct Config { pub data_home: Option<PathBuf>, pub codex: Res<Vec<String>>, pub agy: Res<Vec<String>>, pub wsl: Option<Vec<String>>, pub on_windows: bool }
    impl Config { pub fn from_env() -> Config }   // data_home(), resolve_command AXIOM_CODEX_CMD/codex, AXIOM_AGY_CMD/agy, AXIOM_WSL_CMD/wsl.exe
  pub fn serve(config: Config, input: impl BufRead, output: impl Write + Send + 'static) -> i32
    JSON-RPC 2.0 lines. initialize -> {protocolVersion: the client's or "2025-06-18", capabilities: {tools: {}}, serverInfo: {name: "chronos-bridge", version: CARGO_PKG_VERSION}};
    ping -> {}; tools/list -> {tools: tools()}; tools/call runs on its own thread so long calls never block reads; messages without id are ignored; parse error -32700;
    unknown method -32601. Writes are serialized. Returns 0 at EOF after in-flight calls finish; the app-server is closed on exit.
  pub fn tools() -> Value   // names in this order: codex_run, gemini_preview, orch, review_code, run_on_linux, memory_record, memory_search, memory_get, memory_digest, goal, bridge_status
  pub fn call(bridge: &Bridge, name: &str, args: &Value) -> Value
    Wrapped as {content: [{type: "text", text: <pretty JSON>}], structuredContent: <payload>, isError: payload.status is present and not "completed"}.
    A Failure becomes {status: "bridge_error", error: {kind, message}}; a panic inside a tool is caught the same way.
  pub struct Bridge   // holds Config and one lazily started codex::AppServer
  Tool semantics (input schemas keep the Python properties; additions named here):
  - codex_run: description still says to write `brief` per `${CLAUDE_PLUGIN_ROOT}/DISPATCH.md`. Inputs add `project` (slug), `unit`, `save_to`, `stage`.
    role in terra|sol|astra; model, when given with role, must satisfy codex::role_matches (else invalid_argument); without role a model is required.
    Unit mode (`project` and `unit`): role terra or sol required, `brief` must be absent (the brief comes from the store), sandbox workspace-write.
      astra_required = goal::find_active(project) is Some and goal::astra_requested. orch::begin_unit; Reused returns
      {status: "completed", unit, state: "verified", reused: true}. Otherwise codex run with the brief text, then orch::finish_unit (abort_unit if the run errored before a report).
      Returns {status: "completed" when state is built else "failed", unit, state, model, attempt, verify_exit, verify_tail (only when not built), outside_scope,
      file_changes, usage, report: <md path>, error (kind verify_failed, outside_scope or the codex error)}.
    Astra (`save_to`): requires `project`, role astra, an absolute save_to inside the store; on a completed run with valid structured output writes it there
      (pretty JSON) and orch::record_stage(store, "astra", model, true), else records false and writes nothing. Returns the report without structured_output and
      final_message, plus `saved_to` and `coverage` (structured_output.coverage when present).
    Otherwise the codex report as returned.
  - gemini_preview: as agy_print with default model gemini-4-argon-medium. Inputs add `project`, `stage` (second_review|rewrite) and `save_to` (inside the store):
    with project and stage, records the stage row (ok = completed); with save_to and a completed structured_output, writes it there.
  - orch: {project, args} -> orch::run(store_for(data_home, project), args, json flag from a leading "--json" arg, force from "--force"); creates the parent dir;
    returns {status: completed when exit 0 else failed, store, exit_code, stdout, stderr, error (orch_failed with the stderr/stdout tail) }. In process, no subprocess.
  - review_code: agy::review with default model gemini-4-argon-high. Inputs add `project` and `units`: when given and the review completed,
    orch::record_review(store, units, reviewer model, verdict == "pass") and the result gains `states`.
  - run_on_linux: linux::run. memory_record/search/get/digest: memory::Memory on data_home. bridge_status: codex and agy {command, version: [last line], exit_code}
    or {error}, and codex_app_server_running; no Bun, Java or TLC entries.
  - goal: {session, status?, note?, classify?, mode?}; exactly one of status, classify, mode (else invalid_argument). status -> goal::set_status;
    mode -> goal::set_mode; classify -> goal::classify, and for stop also codex cancel_all and agy::cancel_all. Returns {status: "completed", goal}.
  Acceptance cases (tests/bridge.rs, ports of McpStdioTests through serve with in-memory pipes and the fakes): full session (initialize, tools/list order,
    memory_record of a verified fixture unit then memory_search finds it, codex_run role terra resolves to the newest terra, no model invalid_argument, orch status,
    usage limit isError, gemini_preview echo, bad cwd invalid_argument, bridge_status, unknown method -32601); the codex_run description names DISPATCH.md;
    unit mode end to end in a temp git repo with a registered brief (state built, reports/<unit>.md and .json written, inbox pointer pushed, VERIFY rerun by the bridge);
    second call on a verified unit with unchanged files returns reused without a codex turn; astra_gate refuses unit mode for an [astra] goal until the stage rows and
    files exist; role terra with model gpt-6-sol refused; sol refused before a failing fulltest; save_to for astra writes the file only for an astra model;
    goal classify stop interrupts a SLOW turn in flight; two unit-mode calls on independent units overlap in time; a failed tool never reports isError false.
ACCEPT     tests/bridge.rs, `cargo test --locked --offline --test bridge`

## engine
PURPOSE    The plugin assets (agents, skills, DISPATCH.md, research schemas, hooks.json, plugin.json) moved to engine/, pointed at the binary, cut down to the rules only a prompt can hold, and embedded in the binary.
PATHS      engine/
           src/engine/
           src/axiom/engine/
           .claude-plugin/marketplace.json
           tests/engine.rs
DEPENDS    base, bridge, hook
CONTRACT
  Move with git mv: src/axiom/engine/{DISPATCH.md, agents/, skills/, research/, hooks/hooks.json, .claude-plugin/plugin.json} -> engine/ (same relative paths).
  Delete: src/axiom/engine/bridge/, src/axiom/engine/hooks/goal.py, src/axiom/engine/vendor/ (licenses/pstack-MIT.txt at the repo root stays).
  engine/.claude-plugin/plugin.json: version "0.2.0-beta.1"; mcpServers.bridge = {"command": "${AXIOM_BIN:-axiom}", "args": ["bridge"]}.
  engine/hooks/hooks.json: UserPromptSubmit and Stop run `"${AXIOM_BIN:-axiom}" hook`; PreToolUse with matcher
    "Write|Edit|NotebookEdit|Bash|AskUserQuestion|Agent|Task|mcp__plugin_chronos_bridge__codex_run" runs the same command.
  .claude-plugin/marketplace.json: source "./engine", version "0.2.0-beta.1".
  pub mod engine (src/engine/):
    pub const FILES: &[(&str, &str)]    // (path relative to engine/, include_str! contents) for every file under engine/
    pub fn plugin_dir(home: &Path) -> std::io::Result<PathBuf>
      <home>/engine/<CARGO_PKG_VERSION>/ with every FILES entry written (only when missing or different); returns the directory.
  Text (plan C15): delete each rule now enforced, rewrite steps that name changed interfaces, add no new rule; every file ends shorter than before
    except where a step must name a new argument. Rules that only a prompt can hold keep their wording.
    - DISPATCH.md: delete "After the worker returns" (the bridge reruns VERIFY: orch finish_unit) and "For deletions, SCOPE lists each path to delete." (scope check).
      Keep the brief block, deciding open choices, the negative control and the test-file VERIFY rules.
    - agents/dispatcher.md: register briefs as now (`unit add ... --brief`), run `orch ["map"]` first when a map is given; dispatch with codex_run {project, unit, role, cwd}
      instead of pasting the brief; delete writing reports, inbox push, judging VERIFY, `unit set built/failed`, "If that fails too, leave it failed and continue",
      the unreviewed and never-dismiss sentences, and "Resolve every remaining built unit"; ACCEPT via `orch ["accept", <module>, "--repo", <dir>]`;
      review via review_code with project and units, fallback chronos:reviewer then `orch ["review", ..., "--reviewer", "chronos:reviewer", "--verdict-file", <path>]`;
      full test via `orch ["fulltest", "--repo", <dir>]`; independent ready units (`orch ["ready"]`) are dispatched in one message as parallel codex_run calls.
      Report: `orch ["status"]` output plus the per-module ACCEPT and review lines and each decided choice. Keep module boundaries, memory snippet choice and the sol "more than one unit" condition.
    - agents/architect.md: delete "Order the modules so each depends only on earlier ones and on existing code." (orch map rejects cycles and later dependencies). Keep the rest.
    - agents/astra.md: codex_run gains `project`, `stage` absent and `save_to` the given path (the bridge writes it); delete "Write its structured_output to the given path."
      and "or the model is not an Astra model".
    - agents/reviewer.md: delete "You change nothing." (tools are Read, Glob, Grep) and ", and the verdict is `fail` when any finding is a blocker or major" (orch review derives it).
    - agents/memory-keeper.md: delete the "Only units in state verified count..." sentence and "never read another unit's brief in its place" (memory_record refuses both),
      and "at most 12 lines" (memory_record refuses longer bodies). Keep what is worth keeping.
    - agents/critic.md: unchanged.
    - skills/wish/SKILL.md: delete "You never call Codex or edit the project yourself" and the question count and headless clauses of 1a (hook enforces); keep "only about
      choices that change what gets built" and the decisions.md rule. Add one clause to step 1: declare the task mode with the bridge goal tool (review, answer or change).
      Astra steps pass `project` and `save_to`; gemini_preview second review and the step-5 rewrite pass `project` and `stage`. Keep coverage.read, coverage.not_read,
      "## Coverage", review-failed and unreviewed. Delete 4a's "Continue only when every unit is verified and the full test passes" (goal done gate).
    - skills/research/SKILL.md: the same astra.md and gemini_preview argument changes.
  Acceptance cases (tests/engine.rs, ports of test_engine, test_astra_coverage and the still-valid parts of test_cli/test_dispatch_contract): FILES equals the files on disk under
    engine/; plugin_dir writes them into a temp home and is idempotent; every ${CLAUDE_PLUGIN_ROOT}/ reference exists; every named bridge tool is in bridge::tools() and
    the codex_run description names an existing DISPATCH.md; named chronos:<agent> exist; plan/audit schemas strict and review verdict enum; audit coverage schema with
    a negative control; frontmatter models (architect opus; dispatcher, astra, critic, reviewer sonnet; memory-keeper haiku); hooks.json and plugin.json call `hook`
    and `bridge` through ${AXIOM_BIN:-axiom}; no file mentions AXIOM_PYTHON, bun, vendor or orch.ts; a table in the test lists every deleted sentence above with the
    test that now enforces it (file::test name) and asserts each sentence is absent and each named test function exists in tests/.
ACCEPT     tests/engine.rs, `cargo test --locked --offline --test engine`

## session
PURPOSE    Drive one headless Claude Code session over stream-json in both directions: send the wish, forward every terminal line into the running session at once, keep going while the goal is active, and show progress.
PATHS      src/session/
           tests/session.rs
           tests/fakes/fake_claude.py
DEPENDS    base, goal, hook, engine
CONTRACT
  pub const BRIDGE: &str = "mcp__plugin_chronos_bridge__";
  pub const TOOLS: [&str; 16]   // Agent, Read, Glob, Grep, Write, Bash, then BRIDGE + codex_run, gemini_preview, review_code, run_on_linux, orch, memory_record, memory_search, memory_get, memory_digest, goal
  pub struct Config { pub claude: Res<Vec<String>>, pub home: PathBuf, pub bin: PathBuf }
    impl Config { pub fn from_env() -> Config }   // resolve_command AXIOM_CLAUDE_CMD/claude, base::axiom_home(), std::env::current_exe()
  pub fn command(config: &Config, plugin_dir: &Path, max_turns: Option<u32>) -> Res<Vec<String>>
    <claude> -p --input-format stream-json --output-format stream-json --verbose --plugin-dir <plugin_dir> --setting-sources local --add-dir <home>
    --allowedTools <TOOLS joined by ','> --disallowedTools PowerShell,ScheduleWakeup,CronCreate --model opus [--max-turns N].
    Missing claude: Failure "not_installed" with "claude is not on PATH; install Claude Code and sign in with `claude`".
  pub fn user_message(text: &str) -> String   // {"type":"user","message":{"role":"user","content":[{"type":"text","text":<text>}]}} + "\n"
  pub fn run(config: &Config, skill: &str, text: &str, project: &Path, max_turns: Option<u32>, input: Box<dyn BufRead + Send>, out: &mut dyn Write) -> Res<i32>
    plugin_dir = engine::plugin_dir(home). Spawns claude via base::proc::command in `project` with env AXIOM_DATA_HOME=home, AXIOM_HEADLESS=1, AXIOM_BIN=bin
    (no AXIOM_PYTHON), stdin piped. Writes user_message("/chronos:<skill> <text>") first. A thread reads `input` line by line (CR stripped, blank lines skipped) and writes
    each as a user message immediately; EOF on `input` only stops that thread. Every stdout line is appended to <home>/logs/<UTC yyyymmdd-HHMMSS>-<skill>.jsonl and rendered.
    The session id comes from the system init event. On each result event: an error result ends the session (exit 1); otherwise when a goal for the session is active
    (goal::load) it writes user_message(hook::stop_reason(..)) and continues; otherwise (done, blocked or no goal) it closes claude's stdin and waits.
    Returns 0 after a non-error final result, 1 after an error result, claude's exit code (or 1) when it exits without a result with
    "claude exited <n> without a result; raw events are in <log>". Prints "log: <path>" at the end, and "goal blocked: <note>" when the goal ended blocked.
    Ctrl+C (SetConsoleCtrlHandler on Windows, SIGINT/SIGTERM handler on unix) kills claude's process tree with base::proc::kill_tree, prints the log path and returns 130.
  pub struct Renderer   // pub fn new() -> Renderer; pub fn event(&mut self, event: &Value, out: &mut dyn Write); pub fn summary(&self, out: &mut dyn Write)
    Lines exactly as session.py: "! the plugin:chronos:bridge MCP server is <status>; run `axiom doctor`", "> <agent>: <description>", "< <agent> finished",
    "- codex <role or model>: <GOAL text or first non-blank line or (no brief), max 100 chars>", "- orch <args max 100>", "- <tool>", "- $ <command max 100>",
    nested by two spaces per depth; "astra read: ..." / "astra did not read: ..." from structured_output.coverage or a top-level coverage; each result's text printed in full.
    summary prints once at the end: "<duration_ms/1000>s · $<total_cost_usd>" with "(model $x, ...)" from the LAST result event, never a sum across results;
    "codex tokens: <model> <in> in (<cached> cached) / <out> out" summed per model; Codex and agy costs are never printed as $0.
  tests/fakes/fake_claude.py: records {argv, cwd, env (AXIOM_DATA_HOME, AXIOM_PYTHON, AXIOM_HEADLESS, AXIOM_BIN), messages} to FAKE_CLAUDE_RECORD after every stdin message;
    reads stream-json user messages from stdin; emits init with session_id from FAKE_CLAUDE_SESSION (default a fixed uuid). Scenarios (FAKE_CLAUDE_SCENARIO):
    ok, error, crash, no-bridge as today; interject: emits a codex_run tool_use, waits up to 5 s for a second user message before its tool_result, fails with exit 5 if
    none arrives, then echoes that text in the result; continue: emits a success result with total_cost_usd 0.30, waits for the next user message, writes
    <AXIOM_DATA_HOME>/goals/<session>.json status done, emits a second result with total_cost_usd 0.50, then exits at stdin EOF; it never exits on its own stdin EOF before
    its scenario ends.
  Acceptance cases (tests/session.rs, ports of WishTests and SessionProgressTests through run with Config pointing at the fake): launch loads only the engine
    (argv flags, --model opus, no --max-turns by default, plugin dir under the temp home, cwd, env without AXIOM_PYTHON); first message is "/chronos:wish 幫 todo 加統計功能";
    progress lines and the answer; one log file with the result event; error, crash and no-bridge outcomes; interject: a line given on `input` while the tool runs
    arrives in the same turn (CJK and CRLF input intact) or the test fails on the 5 s timeout; input EOF before the result does not end the session; continue: an active
    goal gets a continuation message, the run ends after the goal is done, and the summary shows $0.50 once (not $0.80); labels, coverage and codex token lines as in
    test_session_progress; command() puts PowerShell only in --disallowedTools.
ACCEPT     tests/session.rs, `cargo test --locked --offline --test session`

## cli
PURPOSE    The `axiom` command line: wish, research, doctor, bridge, hook, orch and memory over the shared module functions, with the beta version string.
PATHS      src/cli/
           tests/cli.rs
DEPENDS    base, orch, memory, hook, bridge, session
CONTRACT
  pub fn main(args: Vec<std::ffi::OsString>) -> i32   // UTF-8 output on every console
  axiom --version                -> "axiom-chronos 0.2.0-beta.1 (beta)", 0
  axiom wish [--project DIR] [--no-architect] [--astra] [--max-turns N] <wish...>
    The first argument that is not one of these options starts the wish; everything after it is wish text (a wish may name --pending). Markers in this order:
    "[no-architect] " then "[astra] ". No --model option. --project defaults to the current directory and must be a directory.
    Empty wish: "axiom: wish: say what you want, e.g. axiom wish add a --stats flag to cli.py", 2. Runs session::run with stdin as input.
  axiom research [--project DIR] [--max-turns N (default 40)] <question...>   // skill "research"
  axiom doctor -> lines "<mark:4> <label:12> <detail>" (marks "ok ", "ERR", "warn"); exit 1 when a required check fails. Checks: claude (required), codex (required),
    agy (optional), sqlite fts5 trigram via rusqlite in memory (required), state directory writable (required), and on Windows pwsh (the Store build is a required
    failure naming `--installer-type wix`) and linux (WSL with ~/.axiom/linux-venv, optional). No Bun, Python, Java or TLC checks.
  axiom bridge -> bridge::serve(Config::from_env(), stdin, stdout)
  axiom hook   -> hook::main_stdin()
  axiom orch [--store DIR | ORCH_STORE] [--json] [--force] <args...> -> prints orch::run stdout/stderr, exits with its code
  axiom memory search <query...> [--project P] [--limit N] | memory get <id> | memory digest <project>   -> pretty JSON on stdout, Failure -> stderr and 1
  Usage errors print "axiom: <message>" to stderr and return 2; a session Failure prints "axiom: <message>" and returns 2.
  Acceptance cases (tests/cli.rs, running env!("CARGO_BIN_EXE_axiom") as a subprocess with AXIOM_HOME, AXIOM_DATA_HOME and the fakes): --version contains "(beta)";
    wish text may name flags; --no-architect and --astra markers in order; research uses the research skill with --max-turns 40; --model is rejected;
    missing claude reported with exit 2; doctor all-present passes and mentions trigram, missing codex gives "ERR  codex", Store pwsh flagged; `bridge` answers initialize
    and tools/list over pipes; `hook` answers a Stop event for an active goal and denies core codex_run; `orch --store <tmp> init` then `unit add` then `status`;
    `memory search` on a fixture db; the binary copied into a directory whose path has a space and CJK characters runs --version, doctor, bridge initialize and hook;
    the median of 20 `axiom hook` cold starts is printed (measured, not asserted). No test needs Python except through the fakes.
ACCEPT     tests/cli.rs, `cargo test --locked --offline --test cli`

## release
PURPOSE    Remove the Python and Bun runtime, run CI on the Rust build, and update the bilingual README and the bridge document.
PATHS      src/axiom/__init__.py
           src/axiom/__main__.py
           src/axiom/cli.py
           src/axiom/session.py
           src/axiom/home.py
           src/axiom/doctor.py
           pyproject.toml
           tests/test_bridge.py
           tests/test_cli.py
           tests/test_goal.py
           tests/test_engine.py
           tests/test_dispatch_contract.py
           tests/test_astra_coverage.py
           tests/test_session_progress.py
           tests/fakes/fake_bun.py
           README.md
           README.zh-TW.md
           docs/
           .github/workflows/ci.yml
           tests/release.rs
DEPENDS    engine, cli
CONTRACT
  Delete every path above except README.md, README.zh-TW.md, docs/, .github/workflows/ci.yml and tests/release.rs; afterwards no *.py remains under src/ and
    pyproject.toml is gone. tests/fakes/*.py other than fake_bun.py stay.
  .github/workflows/ci.yml: on push and pull_request; matrix ubuntu-latest and windows-latest; steps: checkout, actions/setup-python (3.12, for the test fakes),
    the stable Rust toolchain, `cargo build --locked`, `cargo test --locked`, `cargo run --locked -- --version`.
  README.md and README.zh-TW.md: install by building or downloading the one `axiom` binary (`cargo install --git https://github.com/xinhi6716/axiom-chronos --locked`);
    remove Python, pipx and Bun install steps, the --model option and any Sonnet-core example, and the --max-turns 60 default; keep the "beta" label and the version
    0.2.0-beta.1; describe interjection (Esc in interactive mode; in `axiom wish` type a line and press Enter to send it into the running session; amend, answer,
    stop) and `axiom research`. docs/bridge.md: the bridge is `axiom bridge`; the orch row says it runs in process; drop Bun, Java/TLC and Python-version text;
    the tests section points at `cargo test`.
  Acceptance cases (tests/release.rs): no *.py under src/; no pyproject.toml; ci.yml names both runners, `cargo test --locked` and setup-python; neither README mentions
    pipx, bun, AXIOM_PYTHON, "--model" or a 60-turn default; both mention beta, 0.2.0-beta.1 and interjection; docs/bridge.md mentions `axiom bridge` and not orch.ts.
ACCEPT     tests/release.rs, `cargo test --locked --offline --test release`

FULL TEST  `cargo test --locked --offline`
