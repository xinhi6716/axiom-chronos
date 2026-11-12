use super::types::*;
use crate::base::{Failure, Res};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(e: impl std::fmt::Display) -> Failure {
    Failure::new("invalid_argument", e.to_string())
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn canonical(p: &Path) -> String {
    std::fs::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
}
pub fn project_key(project_dir: &Path) -> String {
    let root = canonical(project_dir);
    let git = crate::base::proc::cmd("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let dir = git.map(|s| canonical(&project_dir.join(s))).unwrap_or(root);
    format!("{:x}", Sha256::digest(dir.as_bytes()))[..16].to_string()
}
fn ready_db(db: &Connection) -> Res<bool> {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='g_meta')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !exists {
        return Ok(false);
    }
    Ok(db
        .query_row(
            "SELECT schema_version FROM g_meta WHERE singleton=1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(err)?
        == Some(SCHEMA_VERSION))
}
pub fn migrate(tx: &Transaction<'_>) -> Res<()> {
    migrate_inner(tx, None)
}
fn migrate_inner(tx: &Transaction<'_>, fail: Option<&str>) -> Res<()> {
    if ready_db(tx)? {
        return Ok(());
    }
    tx.execute_batch(include_str!("schema.sql")).map_err(err)?;
    if matches!(fail, Some("schema" | "tables")) {
        return Err(err("injected schema failure"));
    }
    let entries: Vec<(i64, String, String)> = {
        let mut s = tx
            .prepare("SELECT id, project_dir, title FROM entries ORDER BY id")
            .map_err(err)?;
        let v = s
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        v
    };
    for (id, dir, title) in &entries {
        let scope = ensure_scope(tx, Path::new(dir), None)?;
        tx.execute("INSERT INTO g_nodes(scope,key,kind,entry_id,label,summary,attrs,source,state,revision) VALUES(?,?,'entry',?,?,'','{}',NULL,'active',0)",params![scope.id,keys::entry(*id),id,title]).map_err(err)?;
    }
    if matches!(fail, Some("backfill" | "nodes")) {
        return Err(err("injected backfill failure"));
    }
    let count: i64 = tx
        .query_row("SELECT count(*) FROM g_nodes WHERE kind='entry'", [], |r| {
            r.get(0)
        })
        .map_err(err)?;
    if count != entries.len() as i64 {
        return Err(err("entry backfill count mismatch"));
    }
    if matches!(fail, Some("count" | "verify" | "meta")) {
        return Err(err("injected verification failure"));
    }
    tx.execute("INSERT INTO g_meta VALUES(1,?,0)", [SCHEMA_VERSION])
        .map_err(err)?;
    if fail == Some("metadata") {
        return Err(err("injected metadata failure"));
    }
    Ok(())
}
#[doc(hidden)]
pub fn migrate_failing_after(db: &Path, step: &str) -> Res<()> {
    let mut c = Connection::open(db).map_err(err)?;
    c.busy_timeout(std::time::Duration::from_secs(30))
        .map_err(err)?;
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    migrate_inner(&tx, Some(step))?;
    Err(err("unknown failure step"))
}
fn ensure_scope(db: &Connection, dir: &Path, worktree: Option<&Path>) -> Res<Scope> {
    let key = project_key(dir);
    let root = canonical(dir);
    let work = worktree.map(canonical).unwrap_or_default();
    db.execute(
        "INSERT OR IGNORE INTO g_scopes(project_key,root,worktree) VALUES(?,?,?)",
        params![key, root, work],
    )
    .map_err(err)?;
    db.query_row("SELECT id,project_key,root,worktree FROM g_scopes WHERE project_key=? AND root=? AND worktree=?",params![key,root,work],|r|Ok(Scope{id:r.get(0)?,project_key:r.get(1)?,root:r.get(2)?,worktree:r.get(3)?})).map_err(err)
}
pub struct GraphStore {
    db: std::path::PathBuf,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct WriteReport {
    pub nodes_inserted: usize,
    pub nodes_updated: usize,
    pub edges_inserted: usize,
    pub edges_unchanged: usize,
    pub edges_removed: usize,
    pub revision: i64,
}
impl GraphStore {
    pub fn open(home: &Path) -> Res<Self> {
        crate::memory::Memory::open(home)?;
        Ok(Self {
            db: home.join("memory/memory.db"),
        })
    }
    pub fn open_existing(home: &Path) -> Res<Option<Self>> {
        let db = home.join("memory/memory.db");
        Ok(if db.is_file() {
            Some(Self { db })
        } else {
            None
        })
    }
    fn connect(&self) -> Res<Connection> {
        let c = Connection::open_with_flags(&self.db, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(err)?;
        c.busy_timeout(std::time::Duration::from_secs(30))
            .map_err(err)?;
        c.execute_batch("PRAGMA foreign_keys=ON").map_err(err)?;
        Ok(c)
    }
    pub fn ready(&self) -> Res<bool> {
        ready_db(&self.connect()?)
    }
    pub fn scope(&self, p: &Path, w: Option<&Path>) -> Res<Scope> {
        self.with_tx(|tx| ensure_scope(tx, p, w))
    }
    pub fn find_scope(&self, p: &Path, w: Option<&Path>) -> Res<Option<Scope>> {
        let c = self.connect()?;
        c.query_row("SELECT id,project_key,root,worktree FROM g_scopes WHERE project_key=? AND root=? AND worktree=?",params![project_key(p),canonical(p),w.map(canonical).unwrap_or_default()],|r|Ok(Scope{id:r.get(0)?,project_key:r.get(1)?,root:r.get(2)?,worktree:r.get(3)?})).optional().map_err(err)
    }
    pub fn scopes_of(&self, key: &str) -> Res<Vec<Scope>> {
        self.read(|c|{let mut s=c.prepare("SELECT id,project_key,root,worktree FROM g_scopes WHERE project_key=? ORDER BY id").map_err(err)?;let x=s.query_map([key],|r|Ok(Scope{id:r.get(0)?,project_key:r.get(1)?,root:r.get(2)?,worktree:r.get(3)?})).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;Ok(x)})
    }
    pub fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> Res<T>) -> Res<T> {
        let mut c = self.connect()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let before = tx.total_changes();
        let out = f(&tx)?;
        if tx.total_changes() != before {
            tx.execute(
                "UPDATE g_meta SET revision=revision+1 WHERE singleton=1",
                [],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(out)
    }
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Res<T>) -> Res<T> {
        self.process_dirty()?;
        f(&self.connect()?)
    }
    pub fn revision(&self) -> Res<i64> {
        self.connect()?
            .query_row("SELECT revision FROM g_meta WHERE singleton=1", [], |r| {
                r.get(0)
            })
            .map_err(err)
    }
    pub fn process_dirty(&self) -> Res<usize> {
        self.with_tx(process_dirty_tx)
    }
    pub fn write(&self, b: &Batch) -> Res<WriteReport> {
        let mut report = self.with_tx(|tx| write_tx(tx, b))?;
        report.revision = self.revision()?;
        Ok(report)
    }
    pub fn node(&self, r: &NodeRef) -> Res<Option<Node>> {
        self.read(|c| fetch_node(c, "n.scope=?1 AND n.key=?2", params![r.scope, r.key]))
    }
    pub fn node_by_id(&self, id: i64) -> Res<Option<Node>> {
        self.read(|c| fetch_node(c, "n.id=?1", params![id]))
    }
    pub fn node_by_entry(&self, id: i64) -> Res<Option<Node>> {
        self.read(|c| fetch_node(c, "n.entry_id=?1", params![id]))
    }
    pub fn edges(&self, id: i64, dir: Direction, limit: usize) -> Res<Vec<Edge>> {
        self.read(|c|{let cond=match dir {Direction::Out=>"e.src=?1",Direction::In=>"e.dst=?1",Direction::Both=>"(e.src=?1 OR e.dst=?1)"}; let sql=format!("SELECT a.scope,a.key,b.scope,b.key,e.kind,e.locator,e.weight,e.state,e.attrs,s.kind,s.locator,s.sha256,s.version,s.state,s.observed_at FROM g_edges e JOIN g_nodes a ON a.id=e.src JOIN g_nodes b ON b.id=e.dst JOIN g_sources s ON s.id=e.source WHERE {cond} ORDER BY e.kind,e.weight DESC,CASE WHEN e.src=?1 THEN b.key ELSE a.key END LIMIT ?2");let mut s=c.prepare(&sql).map_err(err)?;let rows=s.query_map(params![id,limit as i64],|r|{let attrs:String=r.get(8)?;Ok(Edge{src:NodeRef{scope:r.get(0)?,key:r.get(1)?},dst:NodeRef{scope:r.get(2)?,key:r.get(3)?},kind:r.get(4)?,locator:r.get(5)?,weight:r.get(6)?,state:r.get(7)?,attrs:parse_sql(&attrs)?,source:SourceRow{kind:r.get(9)?,locator:r.get(10)?,sha256:r.get(11)?,version:r.get(12)?,state:r.get(13)?,observed_at:r.get(14)?}})}).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;Ok(rows)})
    }
    pub fn fts(&self, scopes: &[i64], query: &str, limit: usize) -> Res<Vec<(i64, f64)>> {
        if scopes.is_empty() || limit == 0 || query.trim().is_empty() {
            return Ok(vec![]);
        }
        self.read(|c|{let sql="SELECT n.id, rank FROM g_fts JOIN g_nodes n ON n.id=g_fts.rowid WHERE g_fts MATCH ?1 AND n.kind!='entry' AND n.state NOT IN ('tombstone','retired') UNION ALL SELECT n.id, rank FROM entries_fts JOIN entries e ON e.id=entries_fts.rowid JOIN g_nodes n ON n.entry_id=e.id WHERE entries_fts MATCH ?1 AND n.state NOT IN ('tombstone','retired')";let mut s=c.prepare(sql).map_err(err)?;let rows=s.query_map([query],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,f64>(1)?))).map_err(err)?;let mut hits=Vec::new();for row in rows {let (id,rank)=row.map_err(err)?;let scope:i64=c.query_row("SELECT scope FROM g_nodes WHERE id=?",[id],|r|r.get(0)).map_err(err)?;if scopes.contains(&scope){hits.push((id,rank));}} hits.sort_by(|a,b|a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));hits.truncate(limit);Ok(hits)})
    }
    pub fn set_state(&self, r: &NodeRef, state: &str, change: &ChangeRecord) -> Res<()> {
        if !STATES.contains(&state) {
            return Err(invalid("invalid state"));
        }
        self.with_tx(|tx|{let n=tx.execute("UPDATE g_nodes SET state=?,revision=(SELECT revision+1 FROM g_meta) WHERE scope=? AND key=? AND state<>?",params![state,r.scope,r.key,state]).map_err(err)?; if n==0 && !tx.query_row("SELECT EXISTS(SELECT 1 FROM g_nodes WHERE scope=? AND key=?)",params![r.scope,r.key],|r|r.get::<_,bool>(0)).map_err(err)? {return Err(invalid("node not found"))}insert_change(tx,change)?;Ok(())})
    }
    pub fn record_change(&self, c: &ChangeRecord) -> Res<i64> {
        self.with_tx(|tx| insert_change(tx, c))
    }
    pub fn changes(&self, scope: i64, limit: usize) -> Res<Vec<ChangeRecord>> {
        self.read(|c|{let mut s=c.prepare("SELECT scope,actor,op,target,reason,evidence,before,after FROM g_changes WHERE scope=? ORDER BY id DESC LIMIT ?").map_err(err)?;let v=s.query_map(params![scope,limit as i64],|r|Ok(ChangeRecord{scope:r.get(0)?,actor:r.get(1)?,op:r.get(2)?,target:r.get(3)?,reason:r.get(4)?,evidence:parse_sql(&r.get::<_,String>(5)?)?,before:parse_sql(&r.get::<_,String>(6)?)?,after:parse_sql(&r.get::<_,String>(7)?)?})).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;Ok(v)})
    }
}
fn parse_sql(s: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn fetch_node(c: &Connection, cond: &str, p: impl rusqlite::Params) -> Res<Option<Node>> {
    let sql=format!("SELECT n.id,n.scope,n.key,n.kind,CASE WHEN n.kind='entry' THEN COALESCE(e.title,'') ELSE n.label END,CASE WHEN n.kind='entry' THEN COALESCE(e.summary,'') ELSE n.summary END,n.attrs,n.entry_id,n.state,n.revision,s.kind,s.locator,s.sha256,s.version,s.state,s.observed_at FROM g_nodes n LEFT JOIN entries e ON e.id=n.entry_id LEFT JOIN g_sources s ON s.id=n.source WHERE {cond}");
    c.query_row(&sql, p, |r| {
        let kind: Option<String> = r.get(10)?;
        Ok(Node {
            id: r.get(0)?,
            scope: r.get(1)?,
            key: r.get(2)?,
            kind: r.get(3)?,
            label: r.get(4)?,
            summary: r.get(5)?,
            attrs: parse_sql(&r.get::<_, String>(6)?)?,
            entry_id: r.get(7)?,
            state: r.get(8)?,
            revision: r.get(9)?,
            source: match kind {
                Some(kind) => Some(SourceRow {
                    kind,
                    locator: r.get(11)?,
                    sha256: r.get(12)?,
                    version: r.get(13)?,
                    state: r.get(14)?,
                    observed_at: r.get(15)?,
                }),
                None => None,
            },
        })
    })
    .optional()
    .map_err(err)
}
fn process_dirty_tx(tx: &Transaction) -> Res<usize> {
    let entries: Vec<(i64, String, String)> = {
        let mut s=tx.prepare("SELECT e.id,e.project_dir,e.title FROM g_dirty_entries d JOIN entries e ON e.id=d.entry_id").map_err(err)?;
        let v = s
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        v
    };
    for (id, dir, title) in entries {
        let scope = ensure_scope(tx, Path::new(&dir), None)?;
        tx.execute("INSERT INTO g_nodes(scope,key,kind,entry_id,label,summary,attrs,state,revision) VALUES(?,?,'entry',?,?,'','{}','active',(SELECT revision+1 FROM g_meta)) ON CONFLICT(scope,key) DO UPDATE SET entry_id=excluded.entry_id,state='active',label=excluded.label,summary='',revision=excluded.revision WHERE entry_id IS NOT excluded.entry_id OR state<>'active' OR label<>excluded.label OR summary<>''",params![scope.id,keys::entry(id),id,title]).map_err(err)?;
    }
    tx.execute("DELETE FROM g_dirty_entries", []).map_err(err)
}
fn insert_change(tx: &Transaction, c: &ChangeRecord) -> Res<i64> {
    if c.actor.trim().is_empty()
        || c.op.trim().is_empty()
        || c.target.trim().is_empty()
        || !tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM g_scopes WHERE id=?)",
                [c.scope],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
    {
        return Err(invalid("invalid change record"));
    }
    tx.execute("INSERT INTO g_changes(scope,at,actor,op,target,reason,evidence,before,after) VALUES(?,?,?,?,?,?,?,?,?)",params![c.scope,now() as f64,c.actor,c.op,c.target,c.reason,c.evidence.to_string(),c.before.to_string(),c.after.to_string()]).map_err(err)?;
    Ok(tx.last_insert_rowid())
}
fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.starts_with('\\')
        && !p.contains('\\')
        && !p.split('/').any(|x| x == ".." || x.is_empty())
        && !(p.len() >= 2 && p.as_bytes()[1] == b':')
}
fn validate_node(n: &NodeSpec) -> Res<()> {
    if !NODE_KINDS.contains(&n.kind.as_str())
        || !STATES.contains(&n.state.as_str())
        || n.key.is_empty()
        || !n.attrs.is_object()
    {
        return Err(invalid("invalid node"));
    }
    if let Some(p) = n
        .key
        .strip_prefix("file:")
        .or_else(|| n.key.strip_prefix("artifact:"))
        .or_else(|| n.key.strip_prefix("sym:").and_then(|v| v.split('#').next()))
    {
        if !valid_path(p) {
            return Err(invalid("invalid relative path"));
        }
    }
    if n.kind == "entry" && n.entry_id.is_none() {
        return Err(invalid("entry requires entry_id"));
    }
    if n.kind != "entry" && n.entry_id.is_some() {
        return Err(invalid("entry_id requires entry kind"));
    }
    Ok(())
}
pub(crate) fn kind_pair(k: &str, a: &str, b: &str) -> bool {
    EDGE_KINDS
        .iter()
        .any(|(name, src, dst)| *name == k && src.contains(&a) && dst.contains(&b))
}
fn write_tx(tx: &Transaction, b: &Batch) -> Res<WriteReport> {
    if !STATES.contains(&b.source.state.as_str()) || b.source.kind.is_empty() {
        return Err(invalid("invalid source"));
    }
    let project: String = tx
        .query_row(
            "SELECT project_key FROM g_scopes WHERE id=?",
            [b.scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?
        .ok_or_else(|| invalid("unknown scope"))?;
    for n in &b.nodes {
        validate_node(n)?;
    }
    tx.execute("INSERT INTO g_sources(scope,kind,locator,sha256,version,state,observed_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(scope,kind,locator,sha256,version) DO UPDATE SET state=excluded.state WHERE state<>excluded.state",params![b.scope,b.source.kind,b.source.locator,b.source.sha256,b.source.version,b.source.state,now()]).map_err(err)?;
    let source:i64=tx.query_row("SELECT id FROM g_sources WHERE scope=? AND kind=? AND locator=? AND sha256=? AND version=?",params![b.scope,b.source.kind,b.source.locator,b.source.sha256,b.source.version],|r|r.get(0)).map_err(err)?;
    let mut report = WriteReport::default();
    for n in &b.nodes {
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM g_nodes WHERE scope=? AND key=?)",
                params![b.scope, n.key],
                |r| r.get(0),
            )
            .map_err(err)?;
        let (label, summary) = if n.kind == "entry" {
            ("", "")
        } else {
            (n.label.as_str(), n.summary.as_str())
        };
        let count=tx.execute("INSERT INTO g_nodes(scope,key,kind,entry_id,label,summary,attrs,source,state,revision) VALUES(?,?,?,?,?,?,?,?,?,(SELECT revision+1 FROM g_meta)) ON CONFLICT(scope,key) DO UPDATE SET kind=excluded.kind,entry_id=excluded.entry_id,label=excluded.label,summary=excluded.summary,attrs=excluded.attrs,source=excluded.source,state=excluded.state,revision=excluded.revision WHERE kind IS NOT excluded.kind OR entry_id IS NOT excluded.entry_id OR label IS NOT excluded.label OR summary IS NOT excluded.summary OR attrs IS NOT excluded.attrs OR source IS NOT excluded.source OR state IS NOT excluded.state",params![b.scope,n.key,n.kind,n.entry_id,label,summary,n.attrs.to_string(),source,n.state]).map_err(err)?;
        if count > 0 {
            if exists {
                report.nodes_updated += 1
            } else {
                report.nodes_inserted += 1
            }
        }
    }
    let mut keep = std::collections::HashSet::new();
    for e in &b.edges {
        if e.weight > 1000 || !STATES.contains(&e.state.as_str()) || !e.attrs.is_object() {
            return Err(invalid("invalid edge"));
        }
        let endpoint = |r: &NodeRef| -> Res<(i64, String, String)> {
            tx.query_row("SELECT n.id,n.kind,s.project_key FROM g_nodes n JOIN g_scopes s ON s.id=n.scope WHERE n.scope=? AND n.key=?",params![r.scope,r.key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?.ok_or_else(||invalid("edge endpoint missing"))
        };
        let (a, ak, ap) = endpoint(&e.src)?;
        let (d, dk, dp) = endpoint(&e.dst)?;
        if ap != project || dp != project || !kind_pair(&e.kind, &ak, &dk) {
            return Err(invalid("invalid edge endpoints"));
        }
        keep.insert((a, d, e.kind.clone(), e.locator.clone()));
        let changed=tx.execute("INSERT INTO g_edges(src,dst,kind,source,locator,weight,state,attrs) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(src,dst,kind,source,locator) DO UPDATE SET weight=excluded.weight,state=excluded.state,attrs=excluded.attrs WHERE weight<>excluded.weight OR state<>excluded.state OR attrs<>excluded.attrs",params![a,d,e.kind,source,e.locator,e.weight,e.state,e.attrs.to_string()]).map_err(err)?;
        if changed == 0 {
            report.edges_unchanged += 1
        } else {
            report.edges_inserted += 1
        }
    }
    if b.replace_source {
        let old: Vec<(i64, i64, String, String)> = {
            let mut s = tx
                .prepare("SELECT src,dst,kind,locator FROM g_edges WHERE source=?")
                .map_err(err)?;
            let v = s
                .query_map([source], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)?;
            v
        };
        for (a, d, k, l) in old {
            if !keep.contains(&(a, d, k.clone(), l.clone())) {
                report.edges_removed+=tx.execute("DELETE FROM g_edges WHERE src=? AND dst=? AND kind=? AND source=? AND locator=?",params![a,d,k,source,l]).map_err(err)?;
            }
        }
    }
    Ok(report)
}
pub fn apply_typed(
    tx: &Transaction,
    project_dir: &Path,
    entry_id: i64,
    typed: &TypedWrite,
) -> Res<TypedReport> {
    if typed.nodes.len() > TYPED_MAX_NODES
        || typed.links.len() > TYPED_MAX_LINKS
        || serde_json::to_vec(typed).map_err(invalid)?.len() > TYPED_MAX_BYTES
    {
        return Err(invalid("typed write exceeds limit"));
    }
    process_dirty_tx(tx)?;
    let scope = ensure_scope(tx, project_dir, None)?;
    let entry = keys::entry(entry_id);
    let present: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM g_nodes WHERE scope=? AND key=? AND entry_id=?)",
            params![scope.id, entry, entry_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !present {
        return Err(invalid("entry not found in scope"));
    }
    let b = Batch {
        scope: scope.id,
        source: SourceSpec {
            kind: "typed".into(),
            locator: entry.clone(),
            sha256: String::new(),
            version: "1".into(),
            state: "active".into(),
        },
        nodes: typed.nodes.clone(),
        edges: typed
            .links
            .iter()
            .map(|l| EdgeSpec {
                src: NodeRef {
                    scope: scope.id,
                    key: l.from.clone().unwrap_or_else(|| entry.clone()),
                },
                dst: NodeRef {
                    scope: scope.id,
                    key: l.to.clone(),
                },
                kind: l.kind.clone(),
                locator: String::new(),
                weight: 1000,
                state: "active".into(),
                attrs: serde_json::json!({}),
            })
            .collect(),
        replace_source: false,
    };
    write_tx(tx, &b)?;
    Ok(TypedReport {
        nodes: typed.nodes.len(),
        links: typed.links.len(),
    })
}
