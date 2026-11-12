use super::{keys, Direction, GraphStore, Node, NodeRef};
use crate::base::{Failure, Res};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

pub const MAX_NEIGHBOURS: usize = 24;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Neighbour {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub edge: String,
    pub outgoing: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphRef {
    pub key: String,
    pub kind: String,
    pub source: Option<String>,
    pub status: String,
    pub revision: i64,
    pub neighbours: Vec<Neighbour>,
    pub omitted: usize,
}

static READS: AtomicUsize = AtomicUsize::new(0);

#[doc(hidden)]
pub fn reads() -> usize {
    READS.load(Ordering::Relaxed)
}

fn read<T>(f: impl FnOnce() -> Res<T>) -> Res<T> {
    READS.fetch_add(1, Ordering::Relaxed);
    f()
}

fn empty_ref(key: String, kind: &str, status: &str, revision: i64) -> GraphRef {
    GraphRef {
        key,
        kind: kind.into(),
        source: None,
        status: status.into(),
        revision,
        neighbours: Vec::new(),
        omitted: 0,
    }
}

fn refs_for_node(store: &GraphStore, node: Node) -> Res<GraphRef> {
    let edges = read(|| store.edges(node.id, Direction::Both, MAX_NEIGHBOURS))?;
    let total: usize = read(|| {
        store.read(|db| {
            db.query_row(
                "SELECT count(*) FROM g_edges WHERE src=?1 OR dst=?1",
                [node.id],
                |r| r.get(0),
            )
            .map_err(|e| Failure::new("io", e.to_string()))
        })
    })?;
    let mut neighbours = Vec::with_capacity(edges.len());
    for edge in edges {
        let outgoing = edge.src.scope == node.scope && edge.src.key == node.key;
        let other = if outgoing { &edge.dst } else { &edge.src };
        let target = read(|| store.node(other))?
            .ok_or_else(|| Failure::new("io", "missing edge endpoint"))?;
        neighbours.push(Neighbour {
            key: target.key,
            kind: target.kind,
            label: target.label,
            edge: edge.kind,
            outgoing,
        });
    }
    let revision = read(|| store.revision())?;
    Ok(GraphRef {
        key: node.key,
        kind: node.kind,
        source: node.source.map(|s| s.locator),
        status: node.state,
        revision,
        omitted: total.saturating_sub(neighbours.len()),
        neighbours,
    })
}

fn existing(home: &Path) -> Res<Option<GraphStore>> {
    let Some(store) = GraphStore::open_existing(home)? else {
        return Ok(None);
    };
    if read(|| store.ready())? {
        Ok(Some(store))
    } else {
        Ok(None)
    }
}

pub fn memory_refs(home: &Path, entry_ids: &[i64]) -> BTreeMap<i64, GraphRef> {
    let store = match existing(home) {
        Ok(Some(store)) => store,
        Ok(None) => return BTreeMap::new(),
        Err(_) => {
            return entry_ids
                .iter()
                .map(|&id| (id, empty_ref(keys::entry(id), "entry", "error", 0)))
                .collect()
        }
    };
    entry_ids.iter().map(|&id| {
        let key = keys::entry(id);
        let result = (|| -> Res<GraphRef> {
            let mut node = read(|| store.node_by_entry(id))?;
            if node.is_none() {
                let refs: Vec<NodeRef> = read(|| store.read(|db| {
                    let mut q = db.prepare("SELECT scope,key FROM g_nodes WHERE key=?1 AND kind='entry' AND state='tombstone' ORDER BY id").map_err(|e| Failure::new("io", e.to_string()))?;
                    let rows = q.query_map([&key], |r| Ok(NodeRef { scope: r.get(0)?, key: r.get(1)? })).map_err(|e| Failure::new("io", e.to_string()))?;
                    rows.collect::<Result<Vec<_>, _>>().map_err(|e| Failure::new("io", e.to_string()))
                }))?;
                if let Some(reference) = refs.first() { node = read(|| store.node(reference))?; }
            }
            match node {
                Some(node) => refs_for_node(&store, node),
                None => Ok(empty_ref(key.clone(), "entry", "missing", read(|| store.revision())?)),
            }
        })();
        (id, result.unwrap_or_else(|_| empty_ref(key, "entry", "error", 0)))
    }).collect()
}

pub fn module_refs(home: &Path, wish_store: &Path) -> BTreeMap<String, GraphRef> {
    let Some(wish) = wish_store.file_name().and_then(|s| s.to_str()) else {
        return BTreeMap::new();
    };
    let Ok(Some(store)) = existing(home) else {
        return BTreeMap::new();
    };
    let prefix = format!("module:{wish}/");
    let references: Vec<NodeRef> = match read(|| {
        store.read(|db| {
        let mut q = db.prepare("SELECT scope,key FROM g_nodes WHERE kind='module' AND substr(key,1,length(?1))=?1 ORDER BY scope,key").map_err(|e| Failure::new("io", e.to_string()))?;
        let rows = q.query_map([&prefix], |r| Ok(NodeRef { scope: r.get(0)?, key: r.get(1)? })).map_err(|e| Failure::new("io", e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| Failure::new("io", e.to_string()))
    })
    }) {
        Ok(refs) => refs,
        Err(_) => return BTreeMap::new(),
    };
    references
        .into_iter()
        .map(|reference| {
            let key = reference.key.clone();
            let result = read(|| store.node(&reference))
                .and_then(|n| n.ok_or_else(|| Failure::new("io", "missing module")))
                .and_then(|n| refs_for_node(&store, n));
            (
                key.clone(),
                result.unwrap_or_else(|_| empty_ref(key, "module", "error", 0)),
            )
        })
        .collect()
}
