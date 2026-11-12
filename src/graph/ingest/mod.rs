//! Projection of orchestration and research records.
pub mod orch;
pub mod research;
pub use orch::sync_orch;
pub use research::{import_source, SourceLink};

use crate::base::{Failure, Res};
use crate::graph::{Batch, EdgeSpec, GraphStore, IngestReport, NodeRef, NodeSpec, SourceSpec};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

fn hash(bytes: &[u8]) -> String {
    let normalized = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}
fn node(key: String, kind: &str, label: &str, attrs: Value) -> NodeSpec {
    NodeSpec {
        key,
        kind: kind.into(),
        label: label.into(),
        summary: String::new(),
        attrs,
        entry_id: None,
        state: "active".into(),
    }
}
fn edge(scope: i64, a: &str, b: &str, kind: &str, locator: &str, attrs: Value) -> EdgeSpec {
    EdgeSpec {
        src: NodeRef {
            scope,
            key: a.into(),
        },
        dst: NodeRef {
            scope,
            key: b.into(),
        },
        kind: kind.into(),
        locator: locator.into(),
        weight: 1000,
        state: "active".into(),
        attrs,
    }
}
fn batch(scope: i64, kind: &str, locator: String, bytes: &[u8]) -> Batch {
    Batch {
        scope,
        source: SourceSpec {
            kind: kind.into(),
            locator,
            sha256: hash(bytes),
            version: "1".into(),
            state: "active".into(),
        },
        nodes: vec![],
        edges: vec![],
        replace_source: false,
    }
}
fn write(store: &GraphStore, b: &Batch, report: &mut IngestReport) -> Res<()> {
    let r = store.write(b)?;
    report.nodes_written += r.nodes_inserted + r.nodes_updated;
    report.edges_written += r.edges_inserted;
    report.unchanged += r.edges_unchanged;
    Ok(())
}
// Stale only sources belonging to this ingestion namespace, including older hashes.
fn stale(
    store: &GraphStore,
    scope: i64,
    kind: &str,
    prefix: &str,
    current: &HashSet<(String, String)>,
    report: &mut IngestReport,
) -> Res<()> {
    store.with_tx(|tx| {
        let mut q = tx.prepare("SELECT id, locator, sha256 FROM g_sources WHERE scope=? AND kind=? AND state='active'").map_err(|e| Failure::new("io", e.to_string()))?;
        let rows = q.query_map(rusqlite::params![scope, kind], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e| Failure::new("io",e.to_string()))?.collect::<Result<Vec<_>,_>>().map_err(|e| Failure::new("io",e.to_string()))?;
        drop(q);
        for (id, loc, sha) in rows {
            if !loc.starts_with(prefix) || current.contains(&(loc, sha)) { continue; }
            let err = |e: rusqlite::Error| Failure::new("io",e.to_string());
            tx.execute("UPDATE g_sources SET state='stale' WHERE id=?", [id]).map_err(err)?;
            report.removed += tx.execute("UPDATE g_nodes SET state='stale', revision=(SELECT revision+1 FROM g_meta) WHERE source=? AND state='active'", [id]).map_err(err)?;
            report.removed += tx.execute("UPDATE g_edges SET state='stale' WHERE source=? AND state='active'", [id]).map_err(err)?;
        }
        Ok(())
    })
}
fn empty() -> Value {
    json!({})
}
