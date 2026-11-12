use crate::{
    base::{self, Failure, Res},
    codex::{AppServer, RunRequest},
    graph::{store::GraphStore, types::keys},
    memory::lint,
    usage::CodexTokens,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub trait SemanticCheck {
    fn check(&self, prompt: &str) -> Res<(String, Option<CodexTokens>)>;
}
pub struct CodexSemanticCheck<'a> {
    pub server: &'a AppServer,
    pub cwd: PathBuf,
    pub model: String,
}
impl SemanticCheck for CodexSemanticCheck<'_> {
    fn check(&self, prompt: &str) -> Res<(String, Option<CodexTokens>)> {
        let req = RunRequest {
            brief: prompt.into(),
            cwd: self.cwd.to_string_lossy().into_owned(),
            model: self.model.clone(),
            sandbox: "read-only".into(),
            effort: Some("low".into()),
            output_schema: None,
            developer_instructions: None,
            timeout_s: 60,
            lean: true,
            keep_plugins: vec![],
            web_search: false,
        };
        let v = self.server.run_capped(&req, Some(4000))?;
        if v["status"] != "completed" {
            return Err(Failure::new(
                "semantic",
                v["error"]["message"]
                    .as_str()
                    .unwrap_or("Codex check failed"),
            ));
        }
        let u = &v["usage"];
        let usage = u
            .as_object()
            .filter(|m| {
                [
                    "inputTokens",
                    "cachedInputTokens",
                    "outputTokens",
                    "reasoningOutputTokens",
                    "totalTokens",
                ]
                .iter()
                .any(|k| m.get(*k).and_then(Value::as_u64).is_some())
            })
            .map(|_| CodexTokens {
                input: u["inputTokens"].as_u64().unwrap_or(0),
                cached_input: u["cachedInputTokens"].as_u64().unwrap_or(0),
                output: u["outputTokens"].as_u64().unwrap_or(0),
                reasoning_output: u["reasoningOutputTokens"].as_u64().unwrap_or(0),
                total: u["totalTokens"].as_u64().unwrap_or(0),
                missing: ["inputTokens", "cachedInputTokens", "outputTokens"]
                    .map(|k| u[k].as_u64().is_none()),
            });
        Ok((v["final_message"].as_str().unwrap_or("").into(), usage))
    }
}
pub struct DreamOptions<'a> {
    pub project: String,
    pub project_dir: Option<PathBuf>,
    pub dry_run: bool,
    pub max_changes: usize,
    pub deadline: Duration,
    pub semantic: Option<&'a dyn SemanticCheck>,
}
impl Default for DreamOptions<'_> {
    fn default() -> Self {
        Self {
            project: String::new(),
            project_dir: None,
            dry_run: false,
            max_changes: 50,
            deadline: Duration::from_secs(60),
            semantic: None,
        }
    }
}
impl<'a> DreamOptions<'a> {
    pub fn new(project: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DreamChange {
    pub op: String,
    pub target: String,
    pub reason: String,
    pub evidence: Vec<String>,
    pub before: Value,
    pub after: Value,
}
#[derive(Clone, Debug, Serialize)]
pub struct DreamReport {
    pub changes: Vec<DreamChange>,
    pub checked: usize,
    pub unchanged: usize,
    pub stopped: Option<String>,
    pub codex_usage: Option<CodexTokens>,
    pub revision: i64,
    pub dry_run: bool,
}
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn evidence(locator: &str, data: &[u8]) -> Vec<String> {
    vec![format!("{}@{:x}", locator, Sha256::digest(data))]
}
#[derive(Clone)]
struct Entry {
    id: i64,
    node: Option<i64>,
    scope: Option<i64>,
    title: String,
    summary: String,
    body: String,
    files: Vec<String>,
    symbols: Vec<String>,
    dir: PathBuf,
    state: String,
}
#[derive(Clone)]
struct Node {
    id: i64,
    scope: i64,
    key: String,
    kind: String,
    label: String,
    state: String,
}
struct Edge {
    src: i64,
    dst: i64,
    kind: String,
    source: i64,
    locator: String,
    state: String,
}
fn usage_add(sum: &mut Option<CodexTokens>, u: Option<CodexTokens>) {
    if let Some(u) = u {
        if let Some(s) = sum {
            s.input = s.input.saturating_add(u.input);
            s.cached_input = s.cached_input.saturating_add(u.cached_input);
            s.output = s.output.saturating_add(u.output);
            s.reasoning_output = s.reasoning_output.saturating_add(u.reasoning_output);
            s.total = s.total.saturating_add(u.total);
            for i in 0..3 {
                s.missing[i] |= u.missing[i];
            }
        } else {
            *sum = Some(u);
        }
    }
}
fn change(
    op: &str,
    target: String,
    reason: &str,
    before: Value,
    after: Value,
    ev: Vec<String>,
) -> DreamChange {
    DreamChange {
        op: op.into(),
        target,
        reason: reason.into(),
        before,
        after,
        evidence: ev,
    }
}
fn record(tx: &rusqlite::Transaction<'_>, scope: i64, c: &DreamChange) -> Res<()> {
    tx.execute("INSERT INTO g_changes(scope,at,actor,op,target,reason,evidence,before,after) VALUES(?,?,'dream',?,?,?,?,?,?)",params![scope,now(),c.op,c.target,c.reason,serde_json::to_string(&c.evidence).map_err(err)?,c.before.to_string(),c.after.to_string()]).map_err(err)?;
    Ok(())
}
fn source(tx: &rusqlite::Transaction<'_>, scope: i64) -> Res<i64> {
    tx.execute("INSERT OR IGNORE INTO g_sources(scope,kind,locator,sha256,version,state,observed_at) VALUES(?,'dream','dream','','1','active',?)",params![scope,now() as i64]).map_err(err)?;
    tx.query_row("SELECT id FROM g_sources WHERE scope=? AND kind='dream' AND locator='dream' AND sha256='' AND version='1'",[scope],|r|r.get(0)).map_err(err)
}
fn stop(report: &mut DreamReport, opts: &DreamOptions, start: Instant) -> bool {
    if start.elapsed() >= opts.deadline {
        report.stopped = Some("deadline".into());
        true
    } else if report.changes.len() >= opts.max_changes {
        report.stopped = Some("max_changes".into());
        true
    } else {
        false
    }
}
fn emit(
    store: Option<&GraphStore>,
    scope: i64,
    c: DreamChange,
    report: &mut DreamReport,
    f: impl FnOnce(&rusqlite::Transaction<'_>) -> Res<()>,
) -> Res<()> {
    if let Some(store) = store {
        store.with_tx(|tx| {
            f(tx)?;
            record(tx, scope, &c)
        })?;
    }
    report.changes.push(c);
    Ok(())
}

pub fn dream(home: &Path, opts: &DreamOptions<'_>) -> Res<DreamReport> {
    base::require_slug("project", &opts.project)?;
    if opts
        .project_dir
        .as_ref()
        .is_some_and(|p| !p.is_absolute() || !p.is_dir())
    {
        return Err(Failure::new(
            "invalid_argument",
            "project_dir must be an existing absolute directory",
        ));
    }
    let start = Instant::now();
    let mut report = DreamReport {
        changes: vec![],
        checked: 0,
        unchanged: 0,
        stopped: None,
        codex_usage: None,
        revision: 0,
        dry_run: opts.dry_run,
    };
    let dbpath = home.join("memory/memory.db");
    if stop(&mut report, opts, start) {
        if dbpath.exists() {
            let db =
                Connection::open_with_flags(&dbpath, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(err)?;
            report.revision = db
                .query_row("SELECT revision FROM g_meta WHERE singleton=1", [], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(err)?
                .unwrap_or(0);
        }
        return Ok(report);
    }
    if opts.dry_run && !dbpath.exists() {
        let counts = json!({"entries":0,"files":0,"symbols":0});
        report.changes.push(change(
            "index_node",
            keys::project_index(&opts.project),
            "project index counts",
            Value::Null,
            counts.clone(),
            evidence("graph:counts", counts.to_string().as_bytes()),
        ));
        if report.changes.len() >= opts.max_changes {
            report.stopped = Some("max_changes".into());
        }
        return Ok(report);
    }
    let store = if opts.dry_run {
        None
    } else {
        Some(GraphStore::open(home)?)
    };
    if let Some(s) = &store {
        s.process_dirty()?;
    }
    let db = Connection::open_with_flags(&dbpath, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    report.revision = db
        .query_row("SELECT revision FROM g_meta WHERE singleton=1", [], |r| {
            r.get(0)
        })
        .map_err(err)?;
    if stop(&mut report, opts, start) {
        return Ok(report);
    }
    let mut entries: Vec<Entry> = {
        let mut s=db.prepare("SELECT e.id,n.id,n.scope,e.title,e.summary,e.body,e.files,e.symbols,e.project_dir,coalesce(n.state,'active') FROM entries e LEFT JOIN g_nodes n ON n.entry_id=e.id WHERE e.project=? ORDER BY e.id").map_err(err)?;
        let rows = s
            .query_map([&opts.project], |r| {
                let files: String = r.get(6)?;
                let symbols: String = r.get(7)?;
                Ok(Entry {
                    id: r.get(0)?,
                    node: r.get(1)?,
                    scope: r.get(2)?,
                    title: r.get(3)?,
                    summary: r.get(4)?,
                    body: r.get(5)?,
                    files: serde_json::from_str(&files).unwrap_or_default(),
                    symbols: symbols.split_whitespace().map(str::to_owned).collect(),
                    dir: PathBuf::from(r.get::<_, String>(8)?),
                    state: r.get(9)?,
                })
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        rows
    };
    if let Some(dir) = &opts.project_dir {
        let canon = std::fs::canonicalize(dir).map_err(err)?;
        entries
            .retain(|e| std::fs::canonicalize(&e.dir).unwrap_or_else(|_| e.dir.clone()) == canon);
    }
    let dir = opts
        .project_dir
        .clone()
        .or_else(|| entries.first().map(|e| e.dir.clone()))
        .unwrap_or(std::env::current_dir().map_err(err)?);
    let scope: Option<i64> = db
        .query_row(
            "SELECT id FROM g_scopes WHERE project_key=? AND root=? AND worktree=''",
            params![
                crate::graph::store::project_key(&dir),
                std::fs::canonicalize(&dir)
                    .unwrap_or_else(|_| dir.clone())
                    .to_string_lossy()
                    .as_ref()
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    let scope = match (scope, &store) {
        (Some(s), _) => s,
        (None, Some(s)) => s.scope(&dir, None)?.id,
        (None, None) => 0,
    };
    if opts.dry_run {
        for entry in &mut entries {
            if entry.node.is_none() {
                entry.node = Some(-entry.id);
                entry.scope = Some(scope);
            }
        }
    }
    let nodes: Vec<Node> = {
        let mut s = db
            .prepare("SELECT id,scope,key,kind,label,state FROM g_nodes WHERE scope=? ORDER BY id")
            .map_err(err)?;
        let v = s
            .query_map([scope], |r| {
                Ok(Node {
                    id: r.get(0)?,
                    scope: r.get(1)?,
                    key: r.get(2)?,
                    kind: r.get(3)?,
                    label: r.get(4)?,
                    state: r.get(5)?,
                })
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        v
    };
    let edges: Vec<Edge> = {
        let mut s=db.prepare("SELECT e.src,e.dst,e.kind,e.source,e.locator,e.state FROM g_edges e JOIN g_nodes n ON n.id=e.src WHERE n.scope=? ORDER BY e.src,e.dst,e.kind").map_err(err)?;
        let v = s
            .query_map([scope], |r| {
                Ok(Edge {
                    src: r.get(0)?,
                    dst: r.get(1)?,
                    kind: r.get(2)?,
                    source: r.get(3)?,
                    locator: r.get(4)?,
                    state: r.get(5)?,
                })
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        v
    };
    let indexed_files: Vec<String> = nodes
        .iter()
        .filter(|n| n.kind == "file" && n.state == "active")
        .filter_map(|n| {
            n.key
                .strip_prefix("file:")
                .filter(|p| dir.join(p).is_file())
                .map(str::to_owned)
        })
        .collect();
    let symbols: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.kind == "symbol" && n.state == "active")
        .filter(|n| keys::split_symbol(&n.key).is_some_and(|(p, _, _, _)| dir.join(p).is_file()))
        .collect();
    let mut seen: HashMap<(String, String, String), Entry> = HashMap::new();
    for e in &entries {
        if stop(&mut report, opts, start) {
            break;
        }
        if e.state != "active" {
            continue;
        }
        report.checked += 1;
        if let Some(semantic) = opts.semantic {
            let (_, u) = semantic.check(&format!(
                "Review memory entry {}: {}\n{}",
                e.id, e.title, e.body
            ))?;
            usage_add(&mut report.codex_usage, u);
            if stop(&mut report, opts, start) {
                break;
            }
        }
        let ev = evidence(
            &format!("entry:{}", e.id),
            format!("{}\n{}\n{}", e.title, e.summary, e.body).as_bytes(),
        );
        let sig = (
            lint::normalize(&e.title),
            lint::normalize(&e.summary),
            lint::normalize(&e.body),
        );
        if let Some(first) = seen.get(&sig) {
            if let (Some(n), Some(dst), Some(sc)) = (e.node, first.node, e.scope) {
                let c = change(
                    "merge_duplicate",
                    keys::entry(e.id),
                    "normalized duplicate",
                    json!({"state":e.state}),
                    json!({"state":"merged","merged_into":keys::entry(first.id)}),
                    ev,
                );
                emit(store.as_ref(), sc, c, &mut report, |tx| {
                    let src = source(tx, sc)?;
                    tx.execute("UPDATE g_nodes SET state='merged',revision=(SELECT revision+1 FROM g_meta) WHERE id=?",[n]).map_err(err)?;
                    tx.execute("INSERT OR IGNORE INTO g_edges(src,dst,kind,source,locator,weight,state,attrs) VALUES(?,?,'merged_into',?,'',1000,'active','{}')",params![n,dst,src]).map_err(err)?;
                    Ok(())
                })?;
                continue;
            }
        } else {
            seen.insert(sig, e.clone());
        }
        let mut outcome: Option<(String, String, Vec<String>)> = None;
        for old in &e.files {
            if !e.dir.join(old).is_file() && e.body.contains(old) {
                let name = Path::new(old).file_name();
                let candidates: Vec<String> = indexed_files
                    .iter()
                    .filter(|p| Path::new(p).file_name() == name)
                    .cloned()
                    .collect();
                if !candidates.is_empty() {
                    outcome = Some((old.clone(), "path".into(), candidates));
                    break;
                }
            }
        }
        if outcome.is_none() {
            for old in &e.symbols {
                if e.body.contains(old)
                    && !symbols.iter().any(|n| {
                        n.label == *old
                            || keys::split_symbol(&n.key).is_some_and(|(_, _, q, _)| q == *old)
                    })
                {
                    let short = old.rsplit("::").next().unwrap_or(old);
                    let candidates: Vec<String> = symbols
                        .iter()
                        .filter_map(|n| keys::split_symbol(&n.key).map(|(_, _, q, _)| q))
                        .filter(|q| q.rsplit("::").next() == Some(short))
                        .collect();
                    if !candidates.is_empty() {
                        outcome = Some((old.clone(), "symbol".into(), candidates));
                        break;
                    }
                }
            }
        }
        if let Some((old, kind, candidates)) = outcome {
            if let (Some(n), Some(sc)) = (e.node, e.scope) {
                if candidates.len() == 1 && e.body.matches(&old).count() == 1 {
                    let body = e.body.replacen(&old, &candidates[0], 1);
                    if !body.trim().is_empty()
                        && body.lines().count() <= 12
                        && lint::check_quotes(&body, &e.dir, "text").is_empty()
                    {
                        let summary: String = body
                            .lines()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or(&body)
                            .trim()
                            .chars()
                            .take(200)
                            .collect();
                        let c = change(
                            "fix_error",
                            keys::entry(e.id),
                            &format!("unique {kind} replacement"),
                            json!({"body":e.body}),
                            json!({"body":body}),
                            ev,
                        );
                        emit(store.as_ref(), sc, c, &mut report, |tx| {
                            let at = now();
                            tx.execute("UPDATE entries SET body=?,summary=?,corrected=?,confirmed=? WHERE id=?",params![body,summary,at,at,e.id]).map_err(err)?;
                            tx.execute("DELETE FROM g_dirty_entries WHERE entry_id=?", [e.id])
                                .map_err(err)?;
                            Ok(())
                        })?;
                        continue;
                    }
                }
                let reason = if candidates.len() == 1 && e.body.matches(&old).count() == 1 {
                    "replacement rejected by correction checks"
                } else {
                    "ambiguous replacement"
                };
                let c = change(
                    "mark_unconfirmed",
                    keys::entry(e.id),
                    reason,
                    json!({"state":e.state}),
                    json!({"state":"unconfirmed","candidates":candidates}),
                    ev,
                );
                emit(store.as_ref(), sc, c, &mut report, |tx| {
                    tx.execute("UPDATE g_nodes SET state='unconfirmed',revision=(SELECT revision+1 FROM g_meta) WHERE id=?",[n]).map_err(err)?;
                    Ok(())
                })?;
                continue;
            }
        }
        let corrected_reference: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM g_changes WHERE op='fix_error' AND target=? AND json_extract(after,'$.body')=?)",
            params![keys::entry(e.id), e.body], |r| r.get(0),
        ).map_err(err)?;
        if !e.files.is_empty()
            && e.files.iter().all(|f| !e.dir.join(f).is_file())
            && !(corrected_reference
                && e.files.iter().any(|old| {
                    indexed_files.iter().any(|path| {
                        Path::new(old).file_name() == Path::new(path).file_name()
                            && e.body.contains(path)
                    })
                }))
            && !e.symbols.iter().any(|s| {
                symbols.iter().any(|n| {
                    n.label == *s || keys::split_symbol(&n.key).is_some_and(|(_, _, q, _)| q == *s)
                })
            })
        {
            if let (Some(n), Some(sc)) = (e.node, e.scope) {
                let c = change(
                    "retire_stale",
                    keys::entry(e.id),
                    "recorded files and symbols absent",
                    json!({"state":e.state}),
                    json!({"state":"retired"}),
                    ev,
                );
                emit(store.as_ref(), sc, c, &mut report, |tx| {
                    tx.execute("UPDATE g_nodes SET state='retired',revision=(SELECT revision+1 FROM g_meta) WHERE id=?",[n]).map_err(err)?;
                    Ok(())
                })?;
                continue;
            }
        }
        report.unchanged += 1;
    }
    if report.stopped.is_none() {
        for edge in &edges {
            if stop(&mut report, opts, start) {
                break;
            }
            if edge.state != "active" || edge.kind == "merged_into" {
                continue;
            }
            let gone = |n: &Node| {
                (n.kind == "file" || n.kind == "symbol") && (n.state != "active"
                    || (n.kind == "file"
                        && n.key
                            .strip_prefix("file:")
                            .is_some_and(|p| !dir.join(p).is_file()))
                    || (n.kind == "symbol"
                        && keys::split_symbol(&n.key)
                            .is_some_and(|(p, _, _, _)| !dir.join(p).is_file())))
            };
            let a = nodes.iter().find(|n| n.id == edge.src);
            let b = nodes.iter().find(|n| n.id == edge.dst);
            let (Some(a), Some(b)) = (a, b) else { continue };
            if [a, b]
                .iter()
                .any(|n| n.kind == "entry" && matches!(n.state.as_str(), "merged" | "retired"))
            {
                continue;
            }
            let (old, source_end) = if gone(a) {
                (a, true)
            } else if gone(b) {
                (b, false)
            } else {
                continue;
            };
            let candidates: Vec<&Node> = nodes
                .iter()
                .filter(|n| {
                    n.id != old.id
                        && n.scope == old.scope
                        && n.kind == old.kind
                        && n.label == old.label
                        && !gone(n)
                })
                .collect();
            let ev = evidence(
                &format!("edge:{}:{}:{}", a.key, edge.kind, b.key),
                format!("{}:{}:{}", a.key, edge.kind, b.key).as_bytes(),
            );
            let target = format!("{}:{}:{}", a.key, edge.kind, b.key);
            if candidates.len() == 1 {
                let replacement = candidates[0];
                let c = change(
                    "repair_link",
                    target,
                    "unique same-kind same-name endpoint",
                    json!({"src":a.key,"dst":b.key}),
                    json!({"src":if source_end {&replacement.key} else {&a.key},"dst":if source_end {&b.key} else {&replacement.key}}),
                    ev,
                );
                emit(store.as_ref(), scope, c, &mut report, |tx| {
                    tx.execute("UPDATE g_edges SET src=?,dst=? WHERE src=? AND dst=? AND kind=? AND source=? AND locator=?",params![if source_end {replacement.id} else {edge.src},if source_end {edge.dst} else {replacement.id},edge.src,edge.dst,edge.kind,edge.source,edge.locator]).map_err(err)?;
                    Ok(())
                })?;
            } else {
                let c = change(
                    "mark_unconfirmed",
                    target,
                    "missing endpoint has no unique replacement",
                    json!({"state":"active"}),
                    json!({"state":"unconfirmed","candidates":candidates.iter().map(|n|&n.key).collect::<Vec<_>>()}),
                    ev,
                );
                emit(store.as_ref(), scope, c, &mut report, |tx| {
                    tx.execute("UPDATE g_edges SET state='unconfirmed' WHERE src=? AND dst=? AND kind=? AND source=? AND locator=?",params![edge.src,edge.dst,edge.kind,edge.source,edge.locator]).map_err(err)?;
                    Ok(())
                })?;
            }
        }
    }
    if report.stopped.is_none() && !stop(&mut report, opts, start) {
        let counts:Value=db.query_row("SELECT json_object('entries',sum(CASE WHEN n.kind='entry' AND n.state='active' AND e.project=? THEN 1 ELSE 0 END),'files',sum(CASE WHEN n.kind='file' AND n.state='active' THEN 1 ELSE 0 END),'symbols',sum(CASE WHEN n.kind='symbol' AND n.state='active' THEN 1 ELSE 0 END)) FROM g_nodes n LEFT JOIN entries e ON e.id=n.entry_id WHERE n.scope=?",params![opts.project,scope],|r|r.get::<_,String>(0)).map_err(err).and_then(|s|serde_json::from_str(&s).map_err(err))?;
        let counts = json!({"entries":counts["entries"].as_i64().unwrap_or(0),"files":counts["files"].as_i64().unwrap_or(0),"symbols":counts["symbols"].as_i64().unwrap_or(0)});
        let key = keys::project_index(&opts.project);
        let previous: Option<String> = db
            .query_row(
                "SELECT attrs FROM g_nodes WHERE scope=? AND key=?",
                params![scope, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let old = previous
            .as_ref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        if old.as_ref() != Some(&counts) {
            let c = change(
                "index_node",
                key.clone(),
                "project index counts",
                old.unwrap_or(Value::Null),
                counts.clone(),
                evidence("graph:counts", counts.to_string().as_bytes()),
            );
            emit(store.as_ref(), scope, c, &mut report, |tx| {
                let src = source(tx, scope)?;
                tx.execute("INSERT INTO g_nodes(scope,key,kind,label,summary,attrs,source,state,revision) VALUES(?,?,'project_index',?,'',?,?,'active',(SELECT revision+1 FROM g_meta)) ON CONFLICT(scope,key) DO UPDATE SET attrs=excluded.attrs,source=excluded.source,state='active',revision=excluded.revision",params![scope,key,opts.project,counts.to_string(),src]).map_err(err)?;
                Ok(())
            })?;
        }
    }
    if report.stopped.is_none() && report.changes.len() >= opts.max_changes {
        report.stopped = Some("max_changes".into());
    }
    if let Some(s) = &store {
        report.revision = s.revision()?;
    }
    Ok(report)
}

pub fn command(args: &[String]) -> i32 {
    let mut opts = DreamOptions::default();
    let mut json_output = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => opts.dry_run = true,
            "--json" => json_output = true,
            "--project" | "--dir" | "--max-changes" | "--max-seconds" => {
                let flag = &args[i];
                i += 1;
                if i >= args.len() {
                    return 2;
                }
                let v = &args[i];
                match flag.as_str() {
                    "--project" => opts.project = v.clone(),
                    "--dir" => {
                        let p = PathBuf::from(v);
                        if !p.is_absolute() || !p.is_dir() {
                            return 2;
                        }
                        opts.project_dir = Some(p)
                    }
                    "--max-changes" => match v.parse() {
                        Ok(n) => opts.max_changes = n,
                        Err(_) => return 2,
                    },
                    _ => match v.parse::<u64>() {
                        Ok(n) => opts.deadline = Duration::from_secs(n),
                        Err(_) => return 2,
                    },
                }
            }
            _ => return 2,
        }
        i += 1;
    }
    if base::require_slug("project", &opts.project).is_err() {
        return 2;
    }
    match dream(&base::axiom_home(), &opts) {
        Ok(r) => {
            if json_output {
                println!("{}", serde_json::to_string(&r).unwrap_or_default())
            } else {
                println!("{} changes; revision {}", r.changes.len(), r.revision)
            }
            0
        }
        Err(e) => {
            eprintln!("{}: {}", e.kind, e.message);
            1
        }
    }
}
