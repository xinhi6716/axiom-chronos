use super::{
    aliases, budget,
    code::{self, SyncOptions},
    store::GraphStore,
    types::{Direction, Node, NodeRef},
};
use crate::base::{Failure, Res};

use rusqlite::params;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_SEEDS: usize = 6;
pub const MAX_HOPS: u8 = 2;
pub const MAX_NODES: usize = 64;
pub const MAX_EDGES: usize = 128;
pub const DEFAULT_BUDGET: usize = 2400;
pub const HARD_BUDGET: usize = 4096;
pub const MEMORY_BUDGET: usize = 1200;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum View {
    Code,
    Memory,
    Mixed,
}
fn default_view() -> View {
    View::Mixed
}
fn default_direction() -> Direction {
    Direction::Both
}
fn direction<'de, D: Deserializer<'de>>(d: D) -> Result<Direction, D::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Dir {
        Out,
        In,
        Both,
    }
    Ok(match Dir::deserialize(d)? {
        Dir::Out => Direction::Out,
        Dir::In => Direction::In,
        Dir::Both => Direction::Both,
    })
}
fn default_hops() -> u8 {
    MAX_HOPS
}
fn default_budget() -> usize {
    DEFAULT_BUDGET
}
#[derive(Clone, Debug, Deserialize)]
pub struct ExploreRequest {
    pub project_dir: PathBuf,
    pub worktree: Option<PathBuf>,
    pub project: Option<String>,
    pub query: Option<String>,
    pub node: Option<String>,
    #[serde(default = "default_view")]
    pub view: View,
    #[serde(default = "default_direction", deserialize_with = "direction")]
    pub direction: Direction,
    #[serde(default = "default_hops")]
    pub hops: u8,
    #[serde(default = "default_budget")]
    pub budget: usize,
    #[serde(default)]
    pub refresh: bool,
}
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn valid_state(s: &str) -> bool {
    !matches!(
        s,
        "tombstone" | "retired" | "dirty" | "unresolved" | "ambiguous"
    )
}
fn allowed(n: &Node, code_scope: Option<i64>, mem_scope: Option<i64>, view: View) -> bool {
    let code = matches!(n.kind.as_str(), "file" | "symbol");
    valid_state(&n.state)
        && n.source.as_ref().is_none_or(|s| valid_state(&s.state))
        && if code {
            view != View::Memory && Some(n.scope) == code_scope
        } else {
            view != View::Code && Some(n.scope) == mem_scope
        }
}
fn fallback(q: &str) -> Option<&'static str> {
    let mut run = 0;
    for c in q.chars().chain(std::iter::once(' ')) {
        if matches!(c as u32, 0x3400..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x3134f) {
            run += 1;
        } else {
            if (1..=2).contains(&run) {
                return Some("short_cjk");
            }
            run = 0;
        }
    }
    if q.split_whitespace().any(|w| {
        !w.is_empty() && w.chars().count() < 3 && w.chars().all(|c| c.is_alphanumeric() || c == '_')
    }) {
        Some("short_symbol")
    } else {
        None
    }
}
fn path_of(n: &Node) -> Option<&str> {
    n.key
        .strip_prefix("file:")
        .or_else(|| n.key.strip_prefix("sym:").and_then(|s| s.split('#').next()))
}
fn bundle(n: &Node, why: &str, edges: Vec<Value>) -> Value {
    let at = path_of(n).map(|p| format!("{}:{}", p, n.attrs["span"]["line"].as_u64().unwrap_or(1)));
    let summary = if n.summary.is_empty() {
        n.attrs["signature"].as_str().unwrap_or("")
    } else {
        &n.summary
    };
    json!({"key":n.key,"kind":n.kind,"label":n.label,"at":at,"sha256":n.source.as_ref().filter(|s| !s.sha256.is_empty()).map(|s| &s.sha256),"state":n.state,"summary":summary.chars().take(200).collect::<String>(),"why":why,"conditions":n.attrs.get("conditions").filter(|v|v.is_array()).cloned().unwrap_or(json!([])),"contradictions":n.attrs.get("contradictions").filter(|v|v.is_array()).cloned().unwrap_or(json!([])),"edges":edges})
}
fn seed_nodes(
    store: &GraphStore,
    text: &str,
    node: Option<&str>,
    code_scope: Option<i64>,
    mem_scope: Option<i64>,
    view: View,
    fb: Option<&str>,
) -> Res<Vec<Node>> {
    let scopes: Vec<i64> = [code_scope, mem_scope].into_iter().flatten().collect();
    let mut ranked: HashMap<i64, (i32, Node)> = HashMap::new();
    if let Some(key) = node {
        for scope in &scopes {
            if let Some(n) = store.node(&NodeRef {
                scope: *scope,
                key: key.into(),
            })? {
                if allowed(&n, code_scope, mem_scope, view) {
                    ranked.insert(n.id, (0, n));
                }
            }
        }
    }
    if !text.is_empty() && node.is_none() {
        let ids: Vec<i64> = store.read(|c| {
            if fb.is_some() {
                let words: Vec<String> = text.split_whitespace().map(|word| format!("%{}%", word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))).collect();
                let conditions = std::iter::repeat_n("(COALESCE(e.title,n.label)||' '||COALESCE(e.summary,n.summary)||' '||n.key) LIKE ? ESCAPE '\\'", words.len()).collect::<Vec<_>>().join(" OR ");
                let sql = format!("SELECT n.id FROM (SELECT * FROM g_nodes WHERE ((scope=? AND kind IN ('file','symbol')) OR (scope=? AND kind NOT IN ('file','symbol'))) AND state NOT IN ('tombstone','retired','dirty','unresolved','ambiguous') ORDER BY revision DESC,id DESC LIMIT 5000) n LEFT JOIN entries e ON e.id=n.entry_id WHERE ({conditions}) ORDER BY n.revision DESC,n.id DESC");
                let cs = if view == View::Memory { -1 } else { code_scope.unwrap_or(-1) };
                let ms = if view == View::Code { -1 } else { mem_scope.unwrap_or(-1) };
                let mut values = vec![rusqlite::types::Value::Integer(cs), rusqlite::types::Value::Integer(ms)];
                values.extend(words.into_iter().map(rusqlite::types::Value::Text));
                let mut s = c.prepare(&sql).map_err(err)?;
                let rows = s.query_map(rusqlite::params_from_iter(values), |r| r.get(0)).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
                Ok(rows)
            } else {
                let sql = "SELECT n.id FROM g_nodes n LEFT JOIN entries e ON e.id=n.entry_id WHERE lower(COALESCE(e.title,n.label))=lower(?1) OR lower(n.key)=lower(?1) OR (n.kind='file' AND lower(substr(n.key,6))=lower(?1)) ORDER BY n.key,n.id";
                let mut s = c.prepare(sql).map_err(err)?;
                let rows = s.query_map([text], |r| r.get(0)).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
                Ok(rows)
            }
        })?;
        for id in ids {
            if let Some(n) = store.node_by_id(id)? {
                if allowed(&n, code_scope, mem_scope, view) {
                    let exact = n.label.eq_ignore_ascii_case(text)
                        || n.key.eq_ignore_ascii_case(text)
                        || path_of(&n).is_some_and(|p| p.eq_ignore_ascii_case(text));
                    ranked.insert(n.id, (if exact { 0 } else { 2 }, n));
                }
            }
        }
        if fb.is_none() {
            let fts = format!("\"{}\"", text.replace('"', "\"\""));
            for (id, _) in store.fts(&scopes, &fts, 500)? {
                if let Some(n) = store.node_by_id(id)? {
                    if allowed(&n, code_scope, mem_scope, view) {
                        ranked.entry(id).or_insert((2, n));
                    }
                }
            }
        }
    }
    let mut list: Vec<_> = ranked.into_values().collect();
    list.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| {
                if fb.is_some() && a.0 != 0 {
                    b.1.revision.cmp(&a.1.revision).then(b.1.id.cmp(&a.1.id))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then(a.1.key.cmp(&b.1.key))
            .then(a.1.scope.cmp(&b.1.scope))
    });
    list.truncate(MAX_SEEDS);
    Ok(list.into_iter().map(|(_, n)| n).collect())
}
fn ranked_edges(
    store: &GraphStore,
    node: &Node,
    direction: Direction,
) -> Res<Vec<super::types::Edge>> {
    store.read(|c| {
        let cond = match direction { Direction::Out => "e.src=?1", Direction::In => "e.dst=?1", Direction::Both => "(e.src=?1 OR e.dst=?1)" };
        let sql = format!("SELECT a.scope,a.key,b.scope,b.key,e.kind,e.locator,e.weight,e.state,e.attrs,s.kind,s.locator,s.sha256,s.version,s.state,s.observed_at FROM g_edges e JOIN g_nodes a ON a.id=e.src JOIN g_nodes b ON b.id=e.dst JOIN g_sources s ON s.id=e.source WHERE {cond} ORDER BY (e.weight - CASE WHEN ?2 AND e.kind IN ('contains','defines') THEN 500 ELSE 0 END) DESC,e.kind,b.key,a.key LIMIT ?3");
        let mut stmt = c.prepare(&sql).map_err(err)?;
        let rows = stmt.query_map(params![node.id, node.kind == "file", (MAX_EDGES + 1) as i64], |r| {
            let attrs: String = r.get(8)?;
            Ok(super::types::Edge { src: NodeRef { scope: r.get(0)?, key: r.get(1)? }, dst: NodeRef { scope: r.get(2)?, key: r.get(3)? }, kind: r.get(4)?, locator: r.get(5)?, weight: r.get(6)?, state: r.get(7)?, attrs: serde_json::from_str(&attrs).map_err(|e| rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e)))?, source: super::types::SourceRow { kind:r.get(9)?, locator:r.get(10)?, sha256:r.get(11)?, version:r.get(12)?, state:r.get(13)?, observed_at:r.get(14)? } })
        }).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
        Ok(rows)
    })
}
fn dirty_path(n: &Node, project_dir: &Path) -> Option<String> {
    let (path, source) = (path_of(n)?, n.source.as_ref()?);
    if source.kind != "code" {
        return None;
    }
    let sha = std::fs::read(project_dir.join(path))
        .ok()
        .map(|bytes| code::index::sha256_hex(&bytes));
    (sha.as_deref() != Some(source.sha256.as_str())).then(|| path.to_owned())
}

/// Returns the pre-budget walk counts for diagnostics and limit tests.
#[doc(hidden)]
pub fn walk_counts(home: &Path, req: &ExploreRequest) -> Res<(usize, usize)> {
    explore_inner(home, req).map(|(_, counts)| counts)
}

pub fn explore(home: &Path, req: &ExploreRequest) -> Res<Value> {
    explore_inner(home, req).map(|(payload, _)| payload)
}

fn explore_inner(home: &Path, req: &ExploreRequest) -> Res<(Value, (usize, usize))> {
    let limit = req.budget.clamp(256, HARD_BUDGET);
    let mut alias = None;
    let original = req.query.as_deref().unwrap_or("").trim();
    let mut words = Vec::new();
    if let Some(project) = &req.project {
        let a = aliases::load(&home.join("orchestrate").join(project))?;
        for word in original.split_whitespace() {
            let r = a.resolve(word);
            if r.via.is_some() && alias.is_none() {
                alias = Some(r.clone());
            }
            words.push(r.canonical);
        }
    } else {
        words.extend(original.split_whitespace().map(str::to_owned));
    }
    let text = words.join(" ");
    let fb = fallback(&text);
    let store = if req.refresh {
        let s = GraphStore::open(home)?;
        code::sync(
            &s,
            &req.project_dir,
            req.worktree.as_deref(),
            &SyncOptions::default(),
        )?;
        Some(s)
    } else {
        GraphStore::open_existing(home)?
    };
    let mut state = "missing";
    let mut dirty = Vec::<String>::new();
    let mut revision = 0;
    let mut scope_value = Value::Null;
    let mut seeds = Vec::new();
    let mut bundles = Vec::new();
    let mut omitted = 0usize;
    let mut truncated = 0usize;
    let mut counts = (0, 0);
    if let Some(store) = store.filter(|s| s.ready().unwrap_or(false)) {
        let cs = store.find_scope(&req.project_dir, req.worktree.as_deref())?;
        let ms = store.find_scope(&req.project_dir, None)?;
        let mut found = seed_nodes(
            &store,
            &text,
            req.node.as_deref(),
            cs.as_ref().map(|s| s.id),
            ms.as_ref().map(|s| s.id),
            req.view,
            fb,
        )?;
        if found
            .iter()
            .any(|n| dirty_path(n, &req.project_dir).is_some())
        {
            code::sync(
                &store,
                &req.project_dir,
                req.worktree.as_deref(),
                &SyncOptions {
                    deadline: Some(Instant::now() + Duration::from_secs(2)),
                    ..SyncOptions::default()
                },
            )?;
            found = seed_nodes(
                &store,
                &text,
                req.node.as_deref(),
                cs.as_ref().map(|s| s.id),
                ms.as_ref().map(|s| s.id),
                req.view,
                fb,
            )?;
        }
        dirty.extend(found.iter().filter_map(|n| dirty_path(n, &req.project_dir)));
        found.retain(|n| dirty_path(n, &req.project_dir).is_none());
        revision = store.revision()?;
        scope_value = json!({"code":cs.as_ref().map(|s|s.id),"memory":ms.as_ref().map(|s|s.id)});
        if cs.is_some() || ms.is_some() {
            state = if !dirty.is_empty() { "stale" } else { "fresh" };
        }
        let h = code::health(home, &req.project_dir)?;
        if h.state == "partial" {
            state = "partial";
        } else if h.state == "stale" {
            state = "stale";
        }
        seeds = found.iter().map(|n| n.key.clone()).collect();
        let mut q = VecDeque::new();
        let mut seen = HashSet::new();
        for (i, n) in found.into_iter().enumerate() {
            seen.insert(n.id);
            let why = format!("seed {} ({})", i + 1, n.key);
            q.push_back((n, 0u8, why, 10000i32));
        }
        let mut records = Vec::new();
        let mut edge_count = 0usize;
        while let Some((n, hop, why, score)) = q.pop_front() {
            let mut out = Vec::new();
            if hop < req.hops.min(MAX_HOPS) {
                let mut edges = ranked_edges(&store, &n, req.direction)?;
                edges.sort_by(|a, b| {
                    let weight = |e: &super::types::Edge| -> i32 {
                        i32::from(e.weight)
                            - if matches!(e.kind.as_str(), "contains" | "defines")
                                && n.kind == "file"
                            {
                                500
                            } else {
                                0
                            }
                    };
                    weight(b)
                        .cmp(&weight(a))
                        .then(a.kind.cmp(&b.kind))
                        .then(a.dst.key.cmp(&b.dst.key))
                        .then(a.src.key.cmp(&b.src.key))
                });
                for e in edges {
                    if !valid_state(&e.state) || !valid_state(&e.source.state) {
                        continue;
                    }
                    let other = if e.src.scope == n.scope && e.src.key == n.key {
                        &e.dst
                    } else {
                        &e.src
                    };
                    let Some(next) = store.node(other)? else {
                        continue;
                    };
                    if let Some(path) = dirty_path(&next, &req.project_dir) {
                        dirty.push(path);
                        continue;
                    }
                    if !allowed(
                        &next,
                        cs.as_ref().map(|s| s.id),
                        ms.as_ref().map(|s| s.id),
                        req.view,
                    ) {
                        continue;
                    }
                    if edge_count >= MAX_EDGES {
                        truncated += 1;
                        continue;
                    }
                    edge_count += 1;
                    out.push(json!({"kind":e.kind,"to":next.key,"state":e.state}));
                    if !seen.contains(&next.id) {
                        if seen.len() >= MAX_NODES {
                            omitted += 1;
                            continue;
                        }
                        seen.insert(next.id);
                        let next_why = format!("{} --{}--> {}", why, e.kind, next.key);
                        let penalty = if n.kind == "file"
                            && matches!(e.kind.as_str(), "contains" | "defines")
                        {
                            500
                        } else {
                            0
                        };
                        q.push_back((
                            next,
                            hop + 1,
                            next_why,
                            score - 1000 - penalty + i32::from(e.weight) / 10,
                        ));
                    }
                }
            }
            records.push((score, n.key.clone(), bundle(&n, &why, out)));
        }
        counts = (seen.len(), edge_count);
        dirty.sort();
        dirty.dedup();
        if !dirty.is_empty() && state != "partial" {
            state = "stale";
        }
        records.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        bundles = records.into_iter().map(|x| x.2).collect();
    }
    let payload = json!({"status":"completed","error":null,"budget":{"method":"utf8_upper_bound","limit":limit,"used":0,"memory_limit":MEMORY_BUDGET,"memory_used":0},"revision":revision,"scope":scope_value,"query":{"text":original,"alias":alias,"fallback":fb},"freshness":{"state":state,"dirty_files":dirty},"seeds":seeds,"bundles":bundles,"omitted":omitted,"truncated":truncated});
    Ok((budget::fit(payload, limit, MEMORY_BUDGET), counts))
}
