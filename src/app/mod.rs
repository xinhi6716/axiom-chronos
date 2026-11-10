use crate::{
    base,
    goal::{self, Goal},
    orch,
};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod log;
mod store;
pub use log::{AgentView, ModelUsage, OpenCall, Run, Scan, Summary, Tokens};
pub use store::{AstraFiles, Claim, Counts, GoalView, ModuleView, UnitView};

pub const STAGES: [&str; 7] = [
    "astra",
    "critic",
    "architect",
    "build",
    "full_test",
    "memory",
    "report",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NotStarted,
    Running,
    Done,
    Failed,
    Skipped,
}
#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    pub name: &'static str,
    pub status: Status,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct MemoryEntry {
    pub id: i64,
    pub unit: String,
    pub title: String,
    pub created: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Attention {
    pub session: String,
    pub project: Option<String>,
    pub kind: String,
    pub note: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Wish {
    pub session: String,
    pub goal: Goal,
    pub modified: Option<String>,
    /// Newest of the goal file and this wish's logs; an `active` goal whose
    /// activity is old belongs to a process that died without closing it.
    pub last_activity: Option<String>,
    pub store: Option<PathBuf>,
    pub astra: AstraFiles,
    /// Items of the final plan with the modules serving each; empty without a plan.
    pub goals: Vec<GoalView>,
    pub stages: Vec<Stage>,
    pub modules: Vec<ModuleView>,
    pub units: Vec<UnitView>,
    pub counts: Counts,
    pub decisions: Vec<String>,
    /// Memory entries recorded for this wish's project while its logs ran, or for its wish text.
    pub memory: Vec<MemoryEntry>,
    pub runs: Vec<Run>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub home: PathBuf,
    pub taken: String,
    pub live: Option<String>,
    pub wishes: Vec<Wish>,
    pub needs_you: Vec<Attention>,
    pub warnings: Vec<String>,
}

pub struct Poller {
    home: PathBuf,
    budget: u64,
    logs: HashMap<PathBuf, log::State>,
    follow: BTreeSet<String>,
}
impl Poller {
    pub fn new(home: &Path) -> Self {
        Self::with_budget(home, log::BUDGET)
    }
    /// `budget` bounds the bytes read from each followed log per poll.
    pub fn with_budget(home: &Path, budget: u64) -> Self {
        Self {
            home: home.to_path_buf(),
            budget: budget.max(1),
            logs: HashMap::new(),
            follow: BTreeSet::new(),
        }
    }
    /// Scan this session's logs fully as well; active goals are always followed.
    pub fn follow(&mut self, session: &str) {
        self.follow.insert(session.into());
    }
    pub fn poll(&mut self) -> Snapshot {
        let mut warnings = Vec::new();
        let goals = goals(&self.home, &mut warnings);
        let live = goals
            .iter()
            .find(|g| g.goal.status == "active")
            .map(|g| g.session.clone());
        let deep: BTreeSet<String> = goals
            .iter()
            .filter(|g| g.goal.status == "active")
            .map(|g| g.session.clone())
            .chain(self.follow.iter().cloned())
            .collect();
        let runs = log::refresh(
            &mut self.logs,
            &self.home.join("logs"),
            &|s| deep.contains(s),
            self.budget,
            &mut warnings,
        );
        let mut by_session: HashMap<String, Vec<Run>> = HashMap::new();
        for run in runs {
            if let Some(s) = run.session.clone() {
                by_session.entry(s).or_default().push(run);
            }
        }
        let mut stores: HashMap<String, Option<store::View>> = HashMap::new();
        let mut wishes = Vec::new();
        let mut needs_you = Vec::new();
        for g in goals {
            let store = g
                .goal
                .project
                .as_deref()
                .and_then(|p| orch::store_for(&self.home, p).ok());
            let view = match (&g.goal.project, &store) {
                (Some(p), Some(path)) => stores
                    .entry(p.clone())
                    .or_insert_with(|| store::view(path, &mut warnings))
                    .clone(),
                _ => None,
            };
            let mut runs = by_session.remove(&g.session).unwrap_or_default();
            runs.sort_by(|a, b| a.log.cmp(&b.log));
            let scan = runs.last().and_then(|r| r.scan.as_ref());
            let start = runs.iter().filter_map(|r| r.started_unix).min();
            let end = runs
                .iter()
                .filter_map(|r| r.modified_unix)
                .chain(g.modified.and_then(unix))
                .max();
            let window = start.zip(end).map(|(s, e)| (s as f64 - 60.0, e as f64 + 60.0));
            let memory = match &g.goal.project {
                Some(p) => recorded(&self.home, p, &g.goal.wish, window, &mut warnings),
                None => None,
            };
            let stages = stages(&g.goal, view.as_ref(), scan, memory.as_deref());
            attention(&g.session, &g.goal, &mut needs_you);
            let view = view.unwrap_or_else(empty);
            let modified = g.modified.map(iso);
            let last_activity = runs
                .iter()
                .filter_map(|r| r.modified.clone())
                .chain(modified.clone())
                .max();
            wishes.push(Wish {
                session: g.session,
                goal: g.goal,
                modified,
                last_activity,
                store,
                astra: view.astra,
                goals: view.goals,
                stages,
                modules: view.modules,
                units: view.units,
                counts: view.counts,
                decisions: view.decisions,
                memory: memory.unwrap_or_default(),
                runs,
            });
        }
        Snapshot {
            home: self.home.clone(),
            taken: base::now_iso(),
            live,
            wishes,
            needs_you,
            warnings,
        }
    }
}
/// One read-only snapshot; a GUI that polls should keep a `Poller` instead.
pub fn snapshot(home: &Path) -> Snapshot {
    Poller::new(home).poll()
}

struct Loaded {
    session: String,
    goal: Goal,
    modified: Option<SystemTime>,
}
fn goals(home: &Path, warnings: &mut Vec<String>) -> Vec<Loaded> {
    let entries = match std::fs::read_dir(home.join("goals")) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return vec![],
        Err(e) => {
            warnings.push(format!("goals: {e}"));
            return vec![];
        }
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(session) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        match goal::load(home, session) {
            Ok(Some(goal)) => out.push(Loaded {
                session: session.into(),
                goal,
                modified: entry.metadata().and_then(|m| m.modified()).ok(),
            }),
            Ok(None) => {}
            Err(e) => warnings.push(format!("goals/{session}.json: {}", e.message)),
        }
    }
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.session.cmp(&b.session))
    });
    out
}
fn empty() -> store::View {
    store::View {
        goals: vec![],
        map: None,
        map_error: None,
        units: vec![],
        modules: vec![],
        counts: Counts::default(),
        stages: vec![],
        full: vec![],
        decisions: vec![],
        astra: Default::default(),
        verdict: None,
    }
}
fn attention(session: &str, goal: &Goal, out: &mut Vec<Attention>) {
    if let Some(i) = &goal.interjection {
        out.push(Attention {
            session: session.into(),
            project: goal.project.clone(),
            kind: "interjection".into(),
            note: i.prompt.clone(),
        });
    }
    if goal.status == "blocked" || goal.status == "waiting" {
        out.push(Attention {
            session: session.into(),
            project: goal.project.clone(),
            kind: goal.status.clone(),
            note: goal.note.clone().unwrap_or_default(),
        });
    }
}
fn unmarked(wish: &str) -> &str {
    let mut rest = wish.trim_start();
    while rest.starts_with('[') {
        let Some(end) = rest.find(']') else { break };
        rest = rest[end + 1..].trim_start();
    }
    rest.trim()
}
/// Entries recorded for `project` inside the wish's run window, or whose wish text
/// contains (or is contained in) this wish; None when memory.db cannot be read.
fn recorded(
    home: &Path,
    project: &str,
    wish: &str,
    window: Option<(f64, f64)>,
    warnings: &mut Vec<String>,
) -> Option<Vec<MemoryEntry>> {
    let db = home.join("memory").join("memory.db");
    if !db.is_file() {
        return Some(vec![]);
    }
    let result = (|| -> rusqlite::Result<Vec<MemoryEntry>> {
        let conn = rusqlite::Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_millis(250))?;
        let mut stmt = conn.prepare(
            "SELECT id, unit, title, created FROM entries WHERE project = ?1 AND \
             ((?2 IS NOT NULL AND created BETWEEN ?2 AND ?3) OR \
              (?4 <> '' AND (instr(wish, ?4) > 0 OR instr(?4, wish) > 0))) ORDER BY id",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![
                project,
                window.map(|w| w.0),
                window.map(|w| w.1),
                unmarked(wish)
            ],
            |r| {
                let created: f64 = r.get(3)?;
                let created = Duration::try_from_secs_f64(created)
                    .ok()
                    .and_then(|d| UNIX_EPOCH.checked_add(d))
                    .unwrap_or(UNIX_EPOCH);
                Ok(MemoryEntry {
                    id: r.get(0)?,
                    unit: r.get(1)?,
                    title: r.get(2)?,
                    created: iso(created),
                })
            },
        )?;
        rows.collect()
    })();
    match result {
        Ok(entries) => Some(entries),
        Err(e) => {
            warnings.push(format!("memory.db: {e}"));
            None
        }
    }
}
fn stages(
    goal: &Goal,
    store: Option<&store::View>,
    scan: Option<&Scan>,
    memory: Option<&[MemoryEntry]>,
) -> Vec<Stage> {
    use Status::*;
    let open = |kind: &str| {
        scan.is_some_and(|s| s.open.iter().any(|c| c.kind.as_deref() == Some(kind)))
    };
    let finished = |kind: &str| scan.is_some_and(|s| s.finished.get(kind).is_some_and(|n| *n > 0));
    let closed = goal.status != "active";
    let rows = store.map_or(&[][..], |s| s.stages.as_slice());
    let units = store.map_or(&[][..], |s| s.units.as_slice());
    let counts = store.map(|s| s.counts.clone()).unwrap_or_default();
    let mut out = Vec::with_capacity(STAGES.len());
    let mut push = |name: &'static str, status: Status, detail: Option<String>| {
        out.push(Stage {
            name,
            status,
            detail,
        })
    };

    let astra = rows.iter().filter(|r| r.kind == "astra").last();
    let (status, detail) = if open("chronos:astra") {
        (Running, None)
    } else if let Some(r) = astra {
        (
            if r.exit_code == 0 { Done } else { Failed },
            Some(r.command.clone()),
        )
    } else if store.is_some_and(|s| s.astra.plan) {
        (Done, Some("astra/plan.json".into()))
    } else {
        (NotStarted, None)
    };
    push("astra", status, detail);

    let old = rows
        .iter()
        .filter(|r| r.kind == "second_review" || r.kind == "rewrite")
        .last();
    let (status, detail) = if open("chronos:critic") {
        (Running, None)
    } else if let Some(v) = store.and_then(|s| s.verdict.clone()) {
        (Done, Some(v))
    } else if store.is_some_and(|s| s.astra.review) {
        (Done, None)
    } else if let Some(r) = old {
        (
            if r.exit_code == 0 { Done } else { Failed },
            Some(format!("{} {}", r.kind, r.command)),
        )
    } else {
        (NotStarted, None)
    };
    push("critic", status, detail);

    let (status, detail) = if open("chronos:architect") {
        (Running, None)
    } else if let Some(map) = store.and_then(|s| s.map.as_ref()) {
        (Done, Some(format!("{} modules", map.modules.len())))
    } else if let Some(e) = store.and_then(|s| s.map_error.clone()) {
        (Failed, Some(e))
    } else if !units.is_empty() {
        (Skipped, Some("no module map".into()))
    } else if goal.wish.trim_start().starts_with("[no-architect]") {
        (Skipped, Some("[no-architect]".into()))
    } else {
        (NotStarted, None)
    };
    push("architect", status, detail);

    let memory_status = if open("chronos:memory-keeper") {
        Running
    } else if finished("chronos:memory-keeper") || memory.is_some_and(|m| !m.is_empty()) {
        Done
    } else {
        NotStarted
    };
    let progress = format!(
        "{}/{} verified, {} failed, {} built, {} running, {} pending",
        counts.verified,
        counts.total,
        counts.failed,
        counts.unreviewed,
        counts.in_progress,
        counts.pending
    );
    let (status, detail) = if units.is_empty() {
        if open("chronos:dispatcher") {
            (Running, None)
        } else if goal.status == "done" {
            (Skipped, Some("no units".into()))
        } else {
            (NotStarted, None)
        }
    } else if counts.in_progress > 0 || open("chronos:dispatcher") {
        (Running, Some(progress))
    } else if counts.verified == counts.total {
        (Done, Some(progress))
    } else if closed || memory_status != NotStarted {
        (Failed, Some(progress))
    } else {
        (Running, Some(progress))
    };
    push("build", status, detail);

    let full = store.and_then(|s| s.full.iter().filter(|r| r.kind == "full").last());
    let (status, detail) = if open("orch:fulltest") {
        (Running, None)
    } else if let Some(r) = full {
        let newest = units
            .iter()
            .flat_map(|u| u.evidence.iter())
            .map(|e| e.ts.as_str())
            .max()
            .unwrap_or("");
        let stale = if newest > r.ts.as_str() {
            "; stale: units changed since"
        } else {
            ""
        };
        (
            if r.exit_code == 0 { Done } else { Failed },
            Some(format!("exit {} {}{stale}", r.exit_code, r.command)),
        )
    } else if units.is_empty() && goal.status == "done" {
        (Skipped, Some("no units".into()))
    } else {
        (NotStarted, None)
    };
    push("full_test", status, detail);

    push(
        "memory",
        memory_status,
        memory.map(|m| format!("{} entries", m.len())),
    );

    let (status, detail) = match goal.status.as_str() {
        "done" => (Done, None),
        "blocked" | "waiting" => match goal.reason.as_deref() {
            Some(r @ ("interrupted" | "user_stop")) => (Failed, Some(r.into())),
            _ => (Done, Some(goal.status.clone())),
        },
        _ if memory_status != NotStarted => (Running, None),
        _ => (NotStarted, None),
    };
    push("report", status, detail);
    out
}
pub(crate) fn unix(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}
pub(crate) fn iso(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let days = secs.div_euclid(86400);
    let time = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        time / 3600,
        time / 60 % 60,
        time % 60,
        d.subsec_millis()
    )
}
