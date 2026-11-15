use super::SyncOptions;
use crate::{
    base::{Failure, Res},
    graph::{store::GraphStore, types::Scope},
};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct SyncReport {
    pub scope: Scope,
    pub files_seen: usize,
    pub parsed: Vec<String>,
    pub unchanged: usize,
    pub removed: Vec<String>,
    pub reresolved: usize,
    pub skipped: Vec<(String, String)>,
    pub parse_failures: Vec<(String, String)>,
    pub resolved: usize,
    pub ambiguous: usize,
    pub unresolved: usize,
    pub partial: bool,
    pub partial_reason: Option<String>,
    pub revision: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct RefRow {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub name: String,
    pub resolution: String,
    pub target: Option<String>,
    pub candidates: Vec<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Health {
    pub state: String,
    pub parser: String,
    pub schema_version: Option<i64>,
    pub fts_ok: bool,
    pub integrity_ok: bool,
    pub index_version: Option<String>,
    pub last_sync: Option<i64>,
    pub files: usize,
    pub coverage: Option<f64>,
    pub parse_failures: usize,
    pub unresolved: usize,
    pub stale_files: usize,
    pub detail: String,
}
use super::{classify, discover, resolve, sha256_hex, FileInput, PARSER_VERSION, RESOLVER_VERSION};
use crate::graph::code::{
    extract, language_of, symbol_keys, Def, Extraction, Import, ImportName, Language, Ref, Span,
};
use crate::graph::types::keys;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// The public extraction model uses static kind strings.  The persisted representation
// is decoded through owned fields, then checked against the extractor's vocabulary.
#[derive(Deserialize)]
struct SavedSpan {
    line: u32,
    col: u32,
    end_line: u32,
    end_col: u32,
}
impl From<SavedSpan> for Span {
    fn from(s: SavedSpan) -> Self {
        Self {
            line: s.line,
            col: s.col,
            end_line: s.end_line,
            end_col: s.end_col,
        }
    }
}
#[derive(Deserialize)]
struct SavedDef {
    kind: String,
    name: String,
    qualname: String,
    span: SavedSpan,
    signature: String,
    parent: Option<usize>,
    exported: bool,
}
#[derive(Deserialize)]
struct SavedName {
    name: String,
    alias: Option<String>,
}
#[derive(Deserialize)]
struct SavedImport {
    kind: String,
    module: String,
    names: Vec<SavedName>,
    glob: bool,
    default_as: Option<String>,
    namespace_as: Option<String>,
    span: SavedSpan,
}
#[derive(Deserialize)]
struct SavedRef {
    name: String,
    qualifier: Vec<String>,
    kind: String,
    span: SavedSpan,
    enclosing: Option<usize>,
    shadowed: bool,
    dynamic: bool,
}
#[derive(Deserialize)]
struct SavedExtraction {
    language: Option<Language>,
    defs: Vec<SavedDef>,
    imports: Vec<SavedImport>,
    refs: Vec<SavedRef>,
    errors: Vec<String>,
}
fn kind(s: &str) -> Res<&'static str> {
    [
        "function",
        "method",
        "class",
        "struct",
        "enum",
        "trait",
        "interface",
        "type",
        "const",
        "static",
        "variable",
        "module",
        "macro",
        "namespace",
        "use",
        "mod",
        "from",
        "import",
        "es",
        "re_export",
        "require",
        "call",
        "value",
    ]
    .into_iter()
    .find(|x| *x == s)
    .ok_or_else(|| err(format!("unknown persisted kind: {s}")))
}
fn restore(v: Value) -> Res<Extraction> {
    let x: SavedExtraction = serde_json::from_value(v).map_err(err)?;
    Ok(Extraction {
        language: x.language,
        defs: x
            .defs
            .into_iter()
            .map(|d| {
                Ok(Def {
                    kind: kind(&d.kind)?,
                    name: d.name,
                    qualname: d.qualname,
                    span: d.span.into(),
                    signature: d.signature,
                    parent: d.parent,
                    exported: d.exported,
                })
            })
            .collect::<Res<_>>()?,
        imports: x
            .imports
            .into_iter()
            .map(|i| {
                Ok(Import {
                    kind: kind(&i.kind)?,
                    module: i.module,
                    names: i
                        .names
                        .into_iter()
                        .map(|n| ImportName {
                            name: n.name,
                            alias: n.alias,
                        })
                        .collect(),
                    glob: i.glob,
                    default_as: i.default_as,
                    namespace_as: i.namespace_as,
                    span: i.span.into(),
                })
            })
            .collect::<Res<_>>()?,
        refs: x
            .refs
            .into_iter()
            .map(|r| {
                Ok(Ref {
                    name: r.name,
                    qualifier: r.qualifier,
                    kind: kind(&r.kind)?,
                    span: r.span.into(),
                    enclosing: r.enclosing,
                    shadowed: r.shadowed,
                    dynamic: r.dynamic,
                })
            })
            .collect::<Res<_>>()?,
        errors: x.errors,
    })
}
struct File {
    path: String,
    hash: String,
    bytes: usize,
    source: String,
    extraction: Extraction,
    changed: bool,
}
fn expired(o: &SyncOptions) -> bool {
    o.deadline.is_some_and(|d| std::time::Instant::now() >= d)
}
fn partial(r: &mut SyncReport, reason: &str) {
    r.partial = true;
    r.partial_reason = Some(reason.into());
}

fn finish(store: &GraphStore, report: &mut SyncReport, version: &str) -> Res<()> {
    store.with_tx(|tx| {
      tx.execute("INSERT INTO g_sources(scope,kind,locator,sha256,version,state,observed_at) VALUES(?,'code-index','index','',?,'active',?) ON CONFLICT(scope,kind,locator,sha256,version) DO UPDATE SET observed_at=excluded.observed_at",params![report.scope.id,version,now()]).map_err(err)?;
      let source:i64=tx.query_row("SELECT id FROM g_sources WHERE scope=? AND kind='code-index' AND locator='index' AND sha256='' AND version=?",params![report.scope.id,version],|r|r.get(0)).map_err(err)?;
      let attrs=json!({"partial":report.partial,"reason":report.partial_reason,"parse_failures":report.parse_failures.len(),"last_sync":now(),"version":version});
      tx.execute("INSERT INTO g_nodes(scope,key,kind,label,summary,attrs,source,state,revision) VALUES(?,'project_index:code','project_index','Code index','',?,?,?,(SELECT revision+1 FROM g_meta)) ON CONFLICT(scope,key) DO UPDATE SET attrs=excluded.attrs,source=excluded.source,state=excluded.state,revision=excluded.revision",params![report.scope.id,attrs.to_string(),source,if report.partial {"partial"} else {"active"}]).map_err(err)?;
      Ok(())
    })?;
    report.revision = store.revision()?;
    Ok(())
}
pub fn sync(
    store: &GraphStore,
    dir: &Path,
    worktree: Option<&Path>,
    opts: &SyncOptions,
) -> Res<SyncReport> {
    sync_with_versions_and_hook(
        store,
        dir,
        worktree,
        opts,
        PARSER_VERSION,
        RESOLVER_VERSION,
        || {},
    )
}
#[doc(hidden)]
pub fn sync_with_hook<F: FnOnce()>(
    store: &GraphStore,
    dir: &Path,
    worktree: Option<&Path>,
    opts: &SyncOptions,
    hook: F,
) -> Res<SyncReport> {
    sync_with_versions_and_hook(
        store,
        dir,
        worktree,
        opts,
        PARSER_VERSION,
        RESOLVER_VERSION,
        hook,
    )
}
#[doc(hidden)]
pub fn sync_with_versions(
    store: &GraphStore,
    dir: &Path,
    worktree: Option<&Path>,
    opts: &SyncOptions,
    parser: &str,
    resolver: &str,
) -> Res<SyncReport> {
    sync_with_versions_and_hook(store, dir, worktree, opts, parser, resolver, || {})
}
fn sync_with_versions_and_hook<F: FnOnce()>(
    store: &GraphStore,
    dir: &Path,
    worktree: Option<&Path>,
    opts: &SyncOptions,
    parser: &str,
    resolver: &str,
    hook: F,
) -> Res<SyncReport> {
    let scope = store.scope(dir, worktree)?;
    let mut report = SyncReport {
        scope: scope.clone(),
        files_seen: 0,
        parsed: vec![],
        unchanged: 0,
        removed: vec![],
        reresolved: 0,
        skipped: vec![],
        parse_failures: vec![],
        resolved: 0,
        ambiguous: 0,
        unresolved: 0,
        partial: false,
        partial_reason: None,
        revision: 0,
    };
    let version = format!("{parser}|{resolver}");
    let old:BTreeMap<String,(String,String,String)> = store.read(|c| {
        let mut s=c.prepare("SELECT f.path,s.sha256,s.version,n.attrs FROM g_files f JOIN g_sources s ON s.id=f.source JOIN g_nodes n ON n.id=f.node WHERE f.scope=?").map_err(err)?;
        let rows=s.query_map([scope.id],|r|Ok((r.get::<_,String>(0)?,(r.get(1)?,r.get(2)?,r.get(3)?)))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?; Ok(rows)
    })?;
    let (candidates, skipped) = discover(dir).map_err(err)?;
    report.skipped = skipped;
    report.files_seen = candidates.len();
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    if candidates.len() > opts.limits.files {
        partial(&mut report, "files limit");
    }
    for c in candidates {
        seen.insert(c.path.clone());
        if report.partial || expired(opts) {
            if !report.partial {
                partial(&mut report, "deadline");
            }
            break;
        }
        let bytes = match fs::read(&c.abs) {
            Ok(b) => b,
            Err(e) => {
                report.skipped.push((c.path, e.to_string()));
                partial(&mut report, "file read failed");
                break;
            }
        };
        if bytes.len() as u64 > opts.limits.file_bytes {
            report.skipped.push((c.path, "file bytes limit".into()));
            partial(&mut report, "file bytes limit");
            break;
        }
        if let Some(reason) = classify(&bytes) {
            report.skipped.push((c.path, reason.into()));
            partial(&mut report, "file changed during discovery");
            break;
        }
        let hash = sha256_hex(&bytes);
        let byte_count = bytes.len();
        let decoded = String::from_utf8(bytes);
        let unchanged = old
            .get(&c.path)
            .is_some_and(|(h, v, _)| h == &hash && v == &version);
        let (source, encoding_error) = match decoded {
            Ok(s) => (s, None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        let extraction = if unchanged {
            let attrs: Value = serde_json::from_str(&old[&c.path].2).map_err(err)?;
            restore(attrs["extraction"].clone())?
        } else if let Some(message) = encoding_error {
            Extraction {
                language: language_of(&c.path),
                errors: vec![message],
                ..Extraction::default()
            }
        } else {
            let mut extraction = extract(language_of(&c.path).unwrap(), &source);
            if !extraction.errors.is_empty() {
                extraction.defs.clear();
                extraction.imports.clear();
                extraction.refs.clear();
            }
            extraction
        };
        if !extraction.errors.is_empty() {
            report
                .parse_failures
                .push((c.path.clone(), extraction.errors.join("; ")));
        }
        if unchanged {
            report.unchanged += 1
        } else {
            report.parsed.push(c.path.clone())
        }
        files.push(File {
            path: c.path,
            hash,
            bytes: byte_count,
            source,
            extraction,
            changed: !unchanged,
        });
    }
    let removed: Vec<String> = if report.partial {
        vec![]
    } else {
        old.keys().filter(|p| !seen.contains(*p)).cloned().collect()
    };
    let nodes: usize = files.iter().map(|f| 1 + f.extraction.defs.len()).sum();
    let edges: usize = files
        .iter()
        .map(|f| 2 * f.extraction.defs.len() + f.extraction.refs.len() + f.extraction.imports.len())
        .sum();
    if nodes > opts.limits.nodes {
        partial(&mut report, "nodes limit");
    }
    if edges > opts.limits.edges {
        partial(&mut report, "edges limit");
    }
    if report.partial {
        finish(store, &mut report, &version)?;
        return Ok(report);
    }
    hook();
    // Check all hashes while holding the write lock, before replacing any derived row.
    let mut stale = HashSet::new();
    store.with_tx(|tx|{
        for f in &files { if fs::read(dir.join(&f.path)).ok().map(|b|sha256_hex(&b)).as_deref()!=Some(&f.hash) { stale.insert(f.path.clone()); tx.execute("UPDATE g_sources SET state='stale' WHERE id=(SELECT source FROM g_files WHERE scope=? AND path=?)",params![scope.id,f.path]).map_err(err)?; tx.execute("UPDATE g_nodes SET state='stale' WHERE id=(SELECT node FROM g_files WHERE scope=? AND path=?)",params![scope.id,f.path]).map_err(err)?; } }
        if stale.is_empty() { for f in &files {tx.execute("UPDATE g_sources SET state='active' WHERE state='stale' AND id=(SELECT source FROM g_files WHERE scope=? AND path=?)",params![scope.id,f.path]).map_err(err)?;tx.execute("UPDATE g_nodes SET state='active' WHERE state='stale' AND id=(SELECT node FROM g_files WHERE scope=? AND path=?)",params![scope.id,f.path]).map_err(err)?;} }
        Ok(())
    })?;
    if !stale.is_empty() {
        partial(&mut report, "file changed during sync");
        finish(store, &mut report, &version)?;
        return Ok(report);
    }
    // Resolve module imports without evaluating any references.
    let import_extractions: Vec<_> = files
        .iter()
        .map(|f| {
            let mut extraction = f.extraction.clone();
            extraction.refs.clear();
            extraction
        })
        .collect();
    let inputs: Vec<_> = files
        .iter()
        .zip(&import_extractions)
        .map(|(f, e)| FileInput::new(&f.path, e).with_source(&f.source))
        .collect();
    let imports = resolve(&inputs).imports;
    if expired(opts) {
        partial(&mut report, "deadline");
        finish(store, &mut report, &version)?;
        return Ok(report);
    }
    let all_keys: Vec<_> = files
        .iter()
        .map(|f| symbol_keys(&f.path, &f.extraction.defs))
        .collect();
    let changed = files.iter().any(|f| f.changed) || !removed.is_empty();
    if changed {
        // Snapshot dependency information before changed nodes are removed.
        let changed_paths: HashSet<String> = files
            .iter()
            .filter(|f| f.changed)
            .map(|f| f.path.clone())
            .chain(removed.iter().cloned())
            .collect();
        let mut changed_names = HashSet::new();
        for path in &changed_paths {
            if let Some((_, _, attrs)) = old.get(path) {
                let attrs: Value = serde_json::from_str(attrs).map_err(err)?;
                for d in restore(attrs["extraction"].clone())?.defs {
                    changed_names.insert(d.name);
                }
            }
        }
        for f in files.iter().filter(|f| f.changed) {
            changed_names.extend(f.extraction.defs.iter().map(|d| d.name.clone()));
        }
        let live_keys: HashSet<String> = all_keys.iter().flatten().cloned().collect();
        // Keyed by source path and ref locator; retain a removed target's key across syncs.
        let (prior, old_importers) = store.read(|c| {
            let mut prior = BTreeMap::new();
            let mut q = c.prepare("SELECT s.locator,r.locator,r.name,r.kind,r.resolution,n.key,r.candidates FROM g_refs r JOIN g_sources s ON s.id=r.source LEFT JOIN g_nodes n ON n.id=r.target WHERE s.scope=? AND s.kind='code'").map_err(err)?;
            let rows = q.query_map([scope.id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?))).map_err(err)?;
            for row in rows {
                let (path, loc, name, kind, res, target, data) = row.map_err(err)?;
                let data: Value = serde_json::from_str(&data).map_err(err)?;
                let former = target.or_else(|| data["removed_target"].as_str().map(str::to_owned));
                prior.insert((path,loc,name,kind), (res,former));
            }
            let mut importers = HashSet::new();
            let mut q = c.prepare("SELECT s.locator,d.label FROM g_edges e JOIN g_sources s ON s.id=e.source JOIN g_nodes d ON d.id=e.dst WHERE s.scope=? AND s.kind='code' AND e.kind='imports'").map_err(err)?;
            for row in q.query_map([scope.id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(err)? {
                let (importer, target) = row.map_err(err)?;
                if changed_paths.contains(&target) { importers.insert(importer); }
            }
            Ok((prior, importers))
        })?;
        let importers: Vec<bool> = files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                old_importers.contains(&f.path)
                    || imports[i]
                        .iter()
                        .flatten()
                        .any(|&j| changed_paths.contains(&files[j].path))
            })
            .collect();
        let selections: Vec<Vec<bool>> = files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                f.extraction
                    .refs
                    .iter()
                    .map(|r| {
                        let loc = format!("{}:{}:{}", f.path, r.span.line, r.span.col);
                        let prev =
                            prior.get(&(f.path.clone(), loc, r.name.clone(), r.kind.to_owned()));
                        f.changed
                            || importers[i]
                            || prev.is_some_and(|(state, target)| {
                                (matches!(state.as_str(), "unresolved" | "ambiguous")
                                    && changed_names.contains(&r.name))
                                    || (state == "resolved"
                                        && target.as_ref().is_some_and(|k| !live_keys.contains(k)))
                            })
                    })
                    .collect()
            })
            .collect();
        let selected_extractions: Vec<_> = files
            .iter()
            .zip(&selections)
            .map(|(f, selected)| {
                let mut extraction = f.extraction.clone();
                extraction.refs = extraction
                    .refs
                    .into_iter()
                    .zip(selected)
                    .filter_map(|(r, yes)| yes.then_some(r))
                    .collect();
                extraction
            })
            .collect();
        let inputs: Vec<_> = files
            .iter()
            .zip(&selected_extractions)
            .map(|(f, e)| FileInput::new(&f.path, e).with_source(&f.source))
            .collect();
        let resolution = resolve(&inputs);
        if expired(opts) {
            partial(&mut report, "deadline");
            finish(store, &mut report, &version)?;
            return Ok(report);
        }
        // Replace each changed file's structural rows independently, checking its hash under the lock.
        for path in &changed_paths {
            let index = files.iter().position(|f| &f.path == path);
            let committed = store.with_tx(|tx| {
                if let Some(i) = index {
                    let f = &files[i];
                    if fs::read(dir.join(path)).ok().map(|b|sha256_hex(&b)).as_deref()!=Some(&f.hash) {
                        tx.execute("UPDATE g_sources SET state='stale' WHERE id=(SELECT source FROM g_files WHERE scope=? AND path=?)",params![scope.id,path]).map_err(err)?;
                        tx.execute("UPDATE g_nodes SET state='stale' WHERE id=(SELECT node FROM g_files WHERE scope=? AND path=?)",params![scope.id,path]).map_err(err)?;
                        return Ok(false);
                    }
                }
                let source: Option<i64> = tx.query_row("SELECT source FROM g_files WHERE scope=? AND path=?",params![scope.id,path],|r|r.get(0)).optional().map_err(err)?;
                if let Some(source) = source {
                    tx.execute("DELETE FROM g_refs WHERE source=?",[source]).map_err(err)?;
                    tx.execute("DELETE FROM g_edges WHERE source=?",[source]).map_err(err)?;
                    tx.execute("DELETE FROM g_files WHERE scope=? AND path=?",params![scope.id,path]).map_err(err)?;
                    let desired: HashSet<&str> = index.map(|i| all_keys[i].iter().map(String::as_str).collect()).unwrap_or_default();
                    let existing: Vec<(i64,String,String)> = { let mut q=tx.prepare("SELECT id,key,kind FROM g_nodes WHERE scope=? AND source=?").map_err(err)?; let rows=q.query_map(params![scope.id,source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?; rows };
                    for (id,key,kind) in existing { if (kind=="symbol" && !desired.contains(key.as_str())) || (kind=="file" && index.is_none()) { tx.execute("DELETE FROM g_nodes WHERE id=?",[id]).map_err(err)?; } }
                }
                if let Some(i) = index {
                    let f=&files[i];
                    tx.execute("INSERT INTO g_sources(scope,kind,locator,sha256,version,state,observed_at) VALUES(?,'code',?,?,?,'active',?) ON CONFLICT(scope,kind,locator,sha256,version) DO UPDATE SET state='active',observed_at=excluded.observed_at",params![scope.id,f.path,f.hash,version,now()]).map_err(err)?;
                    let sid:i64=tx.query_row("SELECT id FROM g_sources WHERE scope=? AND kind='code' AND locator=? AND sha256=? AND version=?",params![scope.id,f.path,f.hash,version],|r|r.get(0)).map_err(err)?;
                    let attrs=json!({"language":f.extraction.language.unwrap().family(),"bytes":f.bytes,"sha256":f.hash,"extraction":f.extraction});
                    tx.execute("INSERT INTO g_nodes(scope,key,kind,label,summary,attrs,source,state,revision) VALUES(?,?,'file',?,'',?,?,'active',(SELECT revision+1 FROM g_meta)) ON CONFLICT(scope,key) DO UPDATE SET label=excluded.label,attrs=excluded.attrs,source=excluded.source,state='active',revision=excluded.revision",params![scope.id,keys::file(&f.path),f.path,attrs.to_string(),sid]).map_err(err)?;
                    let fid:i64=tx.query_row("SELECT id FROM g_nodes WHERE scope=? AND key=?",params![scope.id,keys::file(&f.path)],|r|r.get(0)).map_err(err)?;
                    tx.execute("INSERT INTO g_files VALUES(?,?,?,?,?,?)",params![scope.id,f.path,fid,sid,f.extraction.language.unwrap().family(),f.bytes as i64]).map_err(err)?;
                    for (d,k) in f.extraction.defs.iter().zip(&all_keys[i]) {
                        tx.execute("INSERT INTO g_nodes(scope,key,kind,label,summary,attrs,source,state,revision) VALUES(?,?,'symbol',?,'',?,?,'active',(SELECT revision+1 FROM g_meta)) ON CONFLICT(scope,key) DO UPDATE SET label=excluded.label,attrs=excluded.attrs,source=excluded.source,state='active',revision=excluded.revision",params![scope.id,k,d.name,json!({"span":d.span,"signature":d.signature}).to_string(),sid]).map_err(err)?;
                    }
                }
                Ok(true)
            })?;
            if !committed {
                partial(&mut report, "file changed during sync");
                finish(store, &mut report, &version)?;
                return Ok(report);
            }
        }
        for (i, f) in files.iter().enumerate() {
            let importer = importers[i];
            let selected = &selections[i];
            if !f.changed && !importer && !selected.iter().any(|x| *x) {
                continue;
            }
            store.with_tx(|tx| {
                let (fid,sid):(i64,i64)=tx.query_row("SELECT node,source FROM g_files WHERE scope=? AND path=?",params![scope.id,f.path],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
                let node=|k:&str| -> Res<i64> {tx.query_row("SELECT id FROM g_nodes WHERE scope=? AND key=?",params![scope.id,k],|r|r.get(0)).map_err(err)};
                let edge=|a:i64,b:i64,k:&str,line:u32| -> Res<()> {tx.execute("INSERT OR IGNORE INTO g_edges VALUES(?,?,?,?,?,1000,'active','{}')",params![a,b,k,sid,format!("{}:{line}",f.path)]).map_err(err)?;Ok(())};
                if f.changed {
                    for (di,d) in f.extraction.defs.iter().enumerate() {let id=node(&all_keys[i][di])?;edge(fid,id,"defines",d.span.line)?;let parent=d.parent.and_then(|p|all_keys[i].get(p)).map(|k|node(k)).transpose()?.unwrap_or(fid);edge(parent,id,"contains",d.span.line)?;}
                }
                if f.changed || importer {
                    tx.execute("DELETE FROM g_edges WHERE source=? AND kind='imports'",[sid]).map_err(err)?;
                    for (ii,imp) in f.extraction.imports.iter().enumerate() {if let Some(j)=imports[i][ii] {edge(fid,node(&keys::file(&files[j].path))?,"imports",imp.span.line)?;}}
                }
                let mut selected_index = 0;
                for (ri,r) in f.extraction.refs.iter().enumerate() {
                    if !selected[ri] {continue;}
                    let outcome=&resolution.refs[i][selected_index];
                    selected_index += 1;
                    let locator=format!("{}:{}:{}",f.path,r.span.line,r.span.col);
                    let former=if f.changed {None} else {prior.get(&(f.path.clone(),locator.clone(),r.name.clone(),r.kind.to_owned())).and_then(|(_,target)|target.as_ref()).filter(|k| !live_keys.contains(*k))};
                    let lost=outcome.resolution!="resolved" && former.is_some();
                    let actual=if lost {"unresolved"} else {outcome.resolution};
                    let target=if lost {None} else {outcome.target.map(|(a,b)|node(&all_keys[a][b])).transpose()?};
                    let owner=r.enclosing.and_then(|d|all_keys[i].get(d)).map(|k|node(k)).transpose()?.unwrap_or(fid);
                    let candidates:Vec<_>=outcome.candidates.iter().map(|&(a,b)|all_keys[a][b].clone()).collect();
                    let reason=if lost {"target removed"} else {&outcome.reason};
                    tx.execute("DELETE FROM g_refs WHERE source=? AND kind=? AND locator=?",params![sid,r.kind,locator]).map_err(err)?;
                    tx.execute("INSERT INTO g_refs(source,owner,name,kind,locator,resolution,target,candidates) VALUES(?,?,?,?,?,?,?,?)",params![sid,owner,r.name,r.kind,locator,actual,target,json!({"keys":candidates,"reason":reason,"removed_target":if lost {former} else {None}}).to_string()]).map_err(err)?;
                    if !f.changed {report.reresolved+=1;}
                }
                // Edges are line-based and may be shared by refs, so rebuild this file's ref edges.
                tx.execute("DELETE FROM g_edges WHERE source=? AND kind IN ('references','calls')",[sid]).map_err(err)?;
                let rows:Vec<(i64,i64,String,String)>={let mut q=tx.prepare("SELECT owner,target,kind,locator FROM g_refs WHERE source=? AND resolution='resolved' AND target IS NOT NULL").map_err(err)?;let rows=q.query_map([sid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;rows};
                for (owner,target,kind,loc) in rows { let line=loc.rsplitn(3,':').nth(1).and_then(|x|x.parse().ok()).unwrap_or(0); edge(owner,target,if kind=="call" && owner!=fid {"calls"} else {"references"},line)?; }
                Ok(())
            })?;
        }
        report.removed = removed;
    }
    store.read(|c| { let mut q=c.prepare("SELECT r.resolution,count(*) FROM g_refs r JOIN g_sources s ON s.id=r.source WHERE s.scope=? AND s.kind='code' GROUP BY r.resolution").map_err(err)?; let rows=q.query_map([scope.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,usize>(1)?))).map_err(err)?;for row in rows {let (k,n)=row.map_err(err)?;match k.as_str() {"resolved"=>report.resolved=n,"ambiguous"=>report.ambiguous=n,"unresolved"=>report.unresolved=n,_=>()}}Ok(())})?;
    finish(store, &mut report, &version)?;
    Ok(report)
}
pub fn refs_at(store: &GraphStore, scope: &Scope, file: &str, line: u32) -> Res<Vec<RefRow>> {
    store.read(|c| {
      let mut s=c.prepare("SELECT r.name,r.locator,r.resolution,n.key,r.candidates FROM g_refs r JOIN g_sources s ON s.id=r.source LEFT JOIN g_nodes n ON n.id=r.target WHERE s.scope=? AND s.kind='code' AND s.locator=?").map_err(err)?;
      let rows=s.query_map(params![scope.id,file],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?))).map_err(err)?;
      let mut out=Vec::new();
      for row in rows {let (name,loc,resolution,target,data)=row.map_err(err)?;let mut p=loc.rsplitn(3,':');let col=p.next().and_then(|x|x.parse().ok()).unwrap_or(0);let l=p.next().and_then(|x|x.parse().ok()).unwrap_or(0);if l!=line {continue} let v:Value=serde_json::from_str(&data).map_err(err)?;let candidates=serde_json::from_value(v.get("keys").cloned().unwrap_or(json!([]))).map_err(err)?;out.push(RefRow{file:file.into(),line:l,col,name,resolution,target,candidates,reason:v["reason"].as_str().unwrap_or("").into()});}
      out.sort_by_key(|r|r.col);Ok(out)
    })
}
pub fn health(home: &Path, project_dir: &Path) -> Res<Health> {
    let mut h = Health {
        state: "missing".into(),
        parser: PARSER_VERSION.into(),
        schema_version: None,
        fts_ok: false,
        integrity_ok: false,
        index_version: None,
        last_sync: None,
        files: 0,
        coverage: None,
        parse_failures: 0,
        unresolved: 0,
        stale_files: 0,
        detail: String::new(),
    };
    let path = home.join("memory/memory.db");
    if !path.is_file() {
        h.detail = "graph database missing".into();
        return Ok(h);
    }
    let result = (|| -> Res<()> {
        let c =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(err)?;
        h.integrity_ok = c
            .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .map_err(err)?
            == "ok";
        h.schema_version = c
            .query_row(
                "SELECT schema_version FROM g_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        h.fts_ok = c.prepare("SELECT rowid FROM g_fts LIMIT 1").is_ok();
        if !h.integrity_ok
            || !h.fts_ok
            || h.schema_version != Some(crate::graph::types::SCHEMA_VERSION)
        {
            h.state = "corrupt".into();
            h.detail = "graph integrity, schema, or FTS check failed".into();
            return Ok(());
        }
        // Project keys use the store's canonical form and group all linked worktrees.
        // The marker revision is monotonic even when syncs occur in the same second.
        let sid: Option<i64> = c.query_row(
            "SELECT s.id FROM g_scopes s JOIN g_nodes n ON n.scope=s.id AND n.key='project_index:code' WHERE s.project_key=? ORDER BY n.revision DESC LIMIT 1",
            [crate::graph::store::project_key(project_dir)], |r| r.get(0),
        ).optional().map_err(err)?;
        let Some(sid) = sid else {
            h.detail = "index missing".into();
            return Ok(());
        };
        let (count,stale):(i64,i64)=c.query_row("SELECT count(*),coalesce(sum(s.state='stale'),0) FROM g_files f JOIN g_sources s ON s.id=f.source WHERE f.scope=?",[sid],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        h.files = count as usize;
        h.stale_files = stale as usize;
        let marker: Option<String> = c
            .query_row(
                "SELECT attrs FROM g_nodes WHERE scope=? AND key='project_index:code'",
                [sid],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let marker: Value = marker
            .map(|m| serde_json::from_str(&m))
            .transpose()
            .map_err(err)?
            .unwrap_or(Value::Null);
        h.index_version = marker["version"].as_str().map(str::to_owned);
        h.last_sync = marker["last_sync"].as_i64();
        h.parse_failures = marker["parse_failures"].as_u64().unwrap_or(0) as usize;
        let (total,unresolved):(i64,i64)=c.query_row("SELECT count(*),coalesce(sum(r.resolution IN ('unresolved','ambiguous')),0) FROM g_refs r JOIN g_sources s ON s.id=r.source WHERE s.scope=? AND s.kind='code'",[sid],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        h.unresolved = unresolved as usize;
        h.coverage = if total > 0 {
            Some((total - unresolved) as f64 / total as f64)
        } else {
            None
        };
        h.state = if !h.integrity_ok
            || !h.fts_ok
            || h.schema_version != Some(crate::graph::types::SCHEMA_VERSION)
        {
            "corrupt"
        } else if h.index_version.is_none() {
            "missing"
        } else if h.stale_files > 0
            || h.index_version.as_deref() != Some(&format!("{PARSER_VERSION}|{RESOLVER_VERSION}"))
        {
            "stale"
        } else if marker["partial"].as_bool() == Some(true) {
            "partial"
        } else {
            "ready"
        }
        .into();
        Ok(())
    })();
    if let Err(e) = result {
        h.state = "corrupt".into();
        h.detail = format!("{e:?}");
    }
    Ok(h)
}
