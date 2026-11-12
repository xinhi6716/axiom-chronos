use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const SCHEMA_VERSION: i64 = 1;
pub const NODE_KINDS: &[&str] = &[
    "project",
    "wish",
    "unit",
    "module",
    "artifact",
    "finding",
    "decision",
    "entry",
    "file",
    "symbol",
    "scope_claim",
    "feedback",
    "project_index",
    "source",
];
pub const STATES: &[&str] = &[
    "active",
    "stale",
    "partial",
    "unconfirmed",
    "retired",
    "merged",
    "tombstone",
    "unresolved",
    "ambiguous",
];
// Relation table: code containment and references, orchestration ownership and
// changes, findings and memory annotations, and related evidence/merge links.
pub const EDGE_KINDS: &[(&str, &[&str], &[&str])] = &[
    ("defines", &["file"], &["symbol"]),
    ("contains", &["file", "symbol"], &["symbol"]),
    ("references", &["file", "symbol"], &["file", "symbol"]),
    ("calls", &["symbol"], &["symbol"]),
    (
        "imports",
        &["file", "symbol"],
        &["file", "symbol", "module"],
    ),
    ("in_module", &["unit"], &["module"]),
    ("depends_on", &["module"], &["module"]),
    ("owns", &["module"], &["file", "artifact", "source"]),
    ("changes", &["unit"], &["file", "symbol"]),
    ("located_in", &["finding"], &["file"]),
    (
        "about",
        &["finding", "entry", "feedback"],
        &["file", "symbol", "unit", "finding"],
    ),
    ("records", &["entry"], &["unit"]),
    (
        "supersedes",
        &["unit", "feedback"],
        &["unit", "wish", "finding", "feedback"],
    ),
    ("fixes", &["unit"], &["unit"]),
    ("claims_path", &["scope_claim"], &["file", "source"]),
    (
        "cites",
        &["finding", "decision", "entry", "feedback", "artifact"],
        &["finding", "decision", "entry", "source", "artifact", "file", "symbol"],
    ),
    ("decided_in", &["finding", "unit", "decision"], &["decision", "unit"]),
    (
        "merged_into",
        &["unit", "finding", "decision"],
        &["unit", "finding", "decision"],
    ),
    (
        "duplicate_of",
        &["unit", "finding", "decision", "entry", "feedback"],
        &["unit", "finding", "decision", "entry", "feedback"],
    ),
];
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Scope {
    pub id: i64,
    pub project_key: String,
    pub root: String,
    pub worktree: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceSpec {
    pub kind: String,
    pub locator: String,
    pub sha256: String,
    pub version: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeSpec {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub summary: String,
    pub attrs: Value,
    pub entry_id: Option<i64>,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NodeRef {
    pub scope: i64,
    pub key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EdgeSpec {
    pub src: NodeRef,
    pub dst: NodeRef,
    pub kind: String,
    pub locator: String,
    pub weight: u16,
    pub state: String,
    pub attrs: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Batch {
    pub scope: i64,
    pub source: SourceSpec,
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
    pub replace_source: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Node {
    pub id: i64,
    pub scope: i64,
    pub key: String,
    pub kind: String,
    pub label: String,
    pub summary: String,
    pub attrs: Value,
    pub entry_id: Option<i64>,
    pub state: String,
    pub revision: i64,
    pub source: Option<SourceRow>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceRow {
    pub kind: String,
    pub locator: String,
    pub sha256: String,
    pub version: String,
    pub state: String,
    pub observed_at: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Edge {
    pub src: NodeRef,
    pub dst: NodeRef,
    pub kind: String,
    pub locator: String,
    pub weight: u16,
    pub state: String,
    pub attrs: Value,
    pub source: SourceRow,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub scope: i64,
    pub actor: String,
    pub op: String,
    pub target: String,
    pub reason: String,
    pub evidence: Value,
    pub before: Value,
    pub after: Value,
}
#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Out,
    In,
    Both,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypedWrite {
    pub nodes: Vec<NodeSpec>,
    pub links: Vec<LinkSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkSpec {
    pub from: Option<String>,
    pub kind: String,
    pub to: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct TypedReport {
    pub nodes: usize,
    pub links: usize,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct IngestReport {
    pub nodes_written: usize,
    pub edges_written: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub skipped: Vec<(String, String)>,
    pub revision: i64,
}
pub const TYPED_MAX_NODES: usize = 32;
pub const TYPED_MAX_LINKS: usize = 64;
pub const TYPED_MAX_BYTES: usize = 65536;
pub mod keys {
    fn path(p: &str) -> String {
        p.replace('\\', "/")
    }
    pub fn file(p: &str) -> String {
        format!("file:{}", path(p))
    }
    pub fn symbol(p: &str, k: &str, q: &str, d: u32) -> String {
        format!(
            "sym:{}#{}:{}{}",
            path(p),
            k,
            q,
            if d >= 2 {
                format!("~{d}")
            } else {
                String::new()
            }
        )
    }
    pub fn split_symbol(key: &str) -> Option<(String, String, String, u32)> {
        let (p, rest) = key.strip_prefix("sym:")?.split_once('#')?;
        let (k, q) = rest.split_once(':')?;
        let (q, d) = match q.rsplit_once('~') {
            Some((q, n)) if n.parse::<u32>().ok().is_some_and(|n| n >= 2) => (q, n.parse().ok()?),
            _ => (q, 1),
        };
        Some((p.into(), k.into(), q.into(), d))
    }
    pub fn entry(id: i64) -> String {
        format!("entry:{id}")
    }
    pub fn wish(w: &str) -> String {
        format!("wish:{w}")
    }
    pub fn module(w: &str, id: &str) -> String {
        format!("module:{w}/{id}")
    }
    pub fn unit(w: &str, id: &str) -> String {
        format!("unit:{w}/{id}")
    }
    pub fn finding(w: &str, u: &str, n: u32) -> String {
        format!("finding:{w}/{u}#{n}")
    }
    pub fn scope_claim(w: &str, u: &str, n: u32) -> String {
        format!("scope_claim:{w}/{u}#{n}")
    }
    pub fn artifact(p: &str) -> String {
        format!("artifact:{}", path(p))
    }
    pub fn decision(w: &str, u: &str, n: u32) -> String {
        format!("decision:{w}/{u}#{n}")
    }
    pub fn feedback(h: &str) -> String {
        format!("feedback:{h}")
    }
    pub fn project_index(p: &str) -> String {
        format!("project_index:{p}")
    }
}
