use crate::{
    base::{Failure, Res},
    graph::{code::index, GraphStore, Scope},
    orch,
};
use rusqlite::params;
use serde::Serialize;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

thread_local! {
    static SURFACE_COMPUTATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Number of surface computations and sync attempts on the current thread.
#[doc(hidden)]
pub fn surface_computations() -> usize {
    SURFACE_COMPUTATIONS.with(Cell::get)
}

fn count_computation() {
    SURFACE_COMPUTATIONS.with(|count| count.set(count.get() + 1));
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Surface {
    pub module: String,
    pub files: BTreeSet<String>,
    pub symbols: BTreeSet<String>,
    pub dependents: BTreeSet<String>,
    pub hotspots: BTreeSet<String>,
    pub state: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Conflict {
    pub a: String,
    pub b: String,
    pub reason: String,
}
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn norm(s: &str) -> String {
    s.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_owned()
}
fn matches_path(repo: &Path, pattern: &str, path: &str) -> bool {
    let p = norm(pattern);
    if p.contains('*') || p.contains('?') {
        let mut re = String::from("^");
        for c in p.chars() {
            match c {
                '*' => re.push_str(".*"),
                '?' => re.push('.'),
                _ => re.push_str(&regex::escape(&c.to_string())),
            }
        }
        re.push('$');
        return regex::Regex::new(&re).is_ok_and(|r| r.is_match(path));
    }
    path == p
        || ((pattern.ends_with('/') || pattern.ends_with('\\') || repo.join(&p).is_dir())
            && path.starts_with(&(p + "/")))
}
fn path_overlap(a: &str, b: &str) -> bool {
    let a = norm(a);
    let b = norm(b);
    let wildcard = a.contains(['*', '?']) || b.contains(['*', '?']);
    let a = a
        .split(['*', '?'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let b = b
        .split(['*', '?'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    a.is_empty()
        || b.is_empty()
        || (wildcard && (a.starts_with(b) || b.starts_with(a)))
        || a == b
        || a.starts_with(&(b.to_owned() + "/"))
        || b.starts_with(&(a.to_owned() + "/"))
}
fn hotspot(path: &str, repo: &Path) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let root = !path.contains('/');
    matches!(
        name.as_str(),
        "cargo.toml"
            | "cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "pyproject.toml"
            | "poetry.lock"
            | "requirements.txt"
            | "go.mod"
            | "go.sum"
            | "gemfile"
            | "gemfile.lock"
            | "lib.rs"
            | "main.rs"
            | "mod.rs"
            | "__init__.py"
            | "index.ts"
            | "index.js"
    ) || name.ends_with(".sql")
        || name.ends_with(".proto")
        || name.contains("schema")
        || name.contains("generated")
        || path.split('/').any(|p| p.eq_ignore_ascii_case("generated"))
        || (root
            && (name.starts_with('.')
                || name.ends_with(".toml")
                || name.ends_with(".yaml")
                || name.ends_with(".yml")
                || name.ends_with(".json")
                || name.ends_with(".config.js")
                || name.ends_with(".config.ts")))
        || fs::read(repo.join(path))
            .ok()
            .is_some_and(|b| index::classify(&b) == Some("generated"))
}
fn disk_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink() {
                continue;
            }
            if t.is_dir() {
                if !matches!(
                    e.file_name().to_str(),
                    Some(".git" | "target" | "node_modules" | "build" | "dist" | ".venv" | "venv")
                ) {
                    walk(root, &p, out);
                }
            } else if t.is_file() {
                if let Ok(s) = p.strip_prefix(root) {
                    out.push(s.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}
fn sym_path(key: &str) -> Option<&str> {
    key.strip_prefix("sym:")
        .and_then(|k| k.split('#').next())
        .or_else(|| key.strip_prefix("file:"))
}
pub fn surface(store: &GraphStore, scope: &Scope, module: &orch::Module) -> Res<Surface> {
    count_computation();
    let repo = Path::new(&scope.root);
    let proven = fresh(store, repo)
        .ok()
        .flatten()
        .is_some_and(|s| s.id == scope.id);
    let mut s = Surface {
        module: module.id.clone(),
        files: BTreeSet::new(),
        symbols: BTreeSet::new(),
        dependents: BTreeSet::new(),
        hotspots: BTreeSet::new(),
        state: if proven { "proven" } else { "unknown" }.into(),
        reason: if proven {
            None
        } else {
            Some("graph stale".into())
        },
    };
    for p in disk_files(repo) {
        if module.paths.iter().any(|x| matches_path(repo, x, &p)) {
            s.files.insert(p);
        }
    }
    store.read(|c| {
        let mut q=c.prepare("SELECT path FROM g_files WHERE scope=?").map_err(err)?;
        for p in q.query_map([scope.id], |r| r.get::<_,String>(0)).map_err(err)? { let p=p.map_err(err)?; if module.paths.iter().any(|x| matches_path(repo,x,&p)) { s.files.insert(p); } }
        let mut q=c.prepare("SELECT key FROM g_nodes WHERE scope=? AND key LIKE 'sym:%' AND state='active'").map_err(err)?;
        for k in q.query_map([scope.id], |r| r.get::<_,String>(0)).map_err(err)? { let k=k.map_err(err)?; if sym_path(&k).is_some_and(|p| s.files.contains(p)) { s.symbols.insert(k); } }
        let mut q=c.prepare("SELECT owner.key, target.key, r.candidates FROM g_refs r JOIN g_sources src ON src.id=r.source JOIN g_nodes owner ON owner.id=r.owner LEFT JOIN g_nodes target ON target.id=r.target WHERE src.scope=? AND r.resolution IN ('resolved','ambiguous')").map_err(err)?;
        let rows=q.query_map([scope.id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?))).map_err(err)?;
        for row in rows { let (owner,target,candidates)=row.map_err(err)?; if sym_path(&owner).is_some_and(|p| s.files.contains(p)) { continue; } let v:serde_json::Value=serde_json::from_str(&candidates).map_err(err)?; let hit=target.as_ref().is_some_and(|k| s.symbols.contains(k)) || v["keys"].as_array().is_some_and(|ks| ks.iter().filter_map(|k| k.as_str()).any(|k| s.symbols.contains(k))); if hit && owner.starts_with("sym:") { s.dependents.insert(owner); } }
        Ok(())
    })?;
    s.hotspots = s
        .files
        .iter()
        .filter(|p| hotspot(p, repo))
        .cloned()
        .collect();
    Ok(s)
}
fn fresh(store: &GraphStore, repo: &Path) -> Res<Option<Scope>> {
    let Some(scope) = store.find_scope(repo, None)? else {
        return Ok(None);
    };
    let (candidates, skipped) = index::discover(repo).map_err(err)?;
    if candidates.is_empty()
        || skipped
            .iter()
            .any(|(_, r)| r != "generated" && r != "binary")
    {
        return Ok(None);
    }
    let data=store.read(|c| {
        let marker: Option<String> = { use rusqlite::OptionalExtension; c.query_row("SELECT attrs FROM g_nodes WHERE scope=? AND key='project_index:code'",[scope.id],|r| r.get(0)).optional().map_err(err)? };
        let mut q=c.prepare("SELECT f.path,s.sha256,s.state FROM g_files f JOIN g_sources s ON s.id=f.source WHERE f.scope=?").map_err(err)?;
        let rows=q.query_map([scope.id],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
        Ok((marker,rows))
    })?;
    let Some(marker) = data.0 else {
        return Ok(None);
    };
    let v: serde_json::Value = serde_json::from_str(&marker).map_err(err)?;
    if v["partial"].as_bool() != Some(false)
        || v["parse_failures"].as_u64().unwrap_or(1) > 0
        || v["version"].as_str()
            != Some(&format!(
                "{}|{}",
                index::PARSER_VERSION,
                index::RESOLVER_VERSION
            ))
    {
        return Ok(None);
    }
    let indexed: BTreeMap<_, _> = data.1.into_iter().map(|(p, h, s)| (p, (h, s))).collect();
    if indexed.len() != candidates.len() {
        return Ok(None);
    }
    for f in candidates {
        let Some((hash, state)) = indexed.get(&f.path) else {
            return Ok(None);
        };
        if state != "active"
            || fs::read(f.abs)
                .ok()
                .map(|b| index::sha256_hex(&b))
                .as_deref()
                != Some(hash)
        {
            return Ok(None);
        }
    }
    Ok(Some(scope))
}
fn result(a: &orch::Module, b: &orch::Module, reason: String) -> Option<Conflict> {
    Some(Conflict {
        a: a.id.clone(),
        b: b.id.clone(),
        reason,
    })
}
static CACHE: OnceLock<Mutex<HashMap<String, Option<Conflict>>>> = OnceLock::new();
pub fn conflict(
    home: &Path,
    repo: &Path,
    a: &orch::Module,
    b: &orch::Module,
) -> Res<Option<Conflict>> {
    conflict_inner(home, repo, a, b, None)
}

#[doc(hidden)]
pub fn conflict_with_sync_options(
    home: &Path,
    repo: &Path,
    a: &orch::Module,
    b: &orch::Module,
    options: &index::SyncOptions,
) -> Res<Option<Conflict>> {
    conflict_inner(home, repo, a, b, Some(options))
}

fn conflict_inner(
    home: &Path,
    repo: &Path,
    a: &orch::Module,
    b: &orch::Module,
    options: Option<&index::SyncOptions>,
) -> Res<Option<Conflict>> {
    if a.shared_owner.as_deref() == Some(&b.id) || b.shared_owner.as_deref() == Some(&a.id) {
        return Ok(None);
    }
    for x in &a.paths {
        for y in &b.paths {
            if path_overlap(x, y) {
                return Ok(result(a, b, format!("path {x}")));
            }
        }
    }
    let initial = index::health(home, repo)
        .ok()
        .map(|h| h.state)
        .unwrap_or_else(|| "missing".into());
    let store = match GraphStore::open(home) {
        Ok(s) => s,
        Err(_) => return Ok(result(a, b, "graph missing".into())),
    };
    let mut scope = fresh(&store, repo).ok().flatten();
    if scope.is_none() {
        count_computation();
        let default_options = index::SyncOptions {
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            ..Default::default()
        };
        let sync = index::sync(&store, repo, None, options.unwrap_or(&default_options));
        match sync {
            Ok(r) if !r.partial && r.parse_failures.is_empty() => {
                scope = fresh(&store, repo).ok().flatten();
            }
            _ => return Ok(result(a, b, "graph stale".into())),
        }
    }
    let Some(scope) = scope else {
        return Ok(result(
            a,
            b,
            if initial == "missing" {
                "graph missing"
            } else {
                "graph stale"
            }
            .into(),
        ));
    };
    let rev = store.revision()?;
    // Manifests and generated files are not necessarily in the code index.
    // Include owned disk content so they cannot leave a cached result stale.
    let mut disk = BTreeMap::new();
    for path in disk_files(repo) {
        if a.paths
            .iter()
            .chain(&b.paths)
            .any(|p| matches_path(repo, p, &path))
        {
            let hash = match fs::read(repo.join(&path)) {
                Ok(bytes) => index::sha256_hex(&bytes),
                Err(_) => return Ok(result(a, b, "graph stale".into())),
            };
            disk.insert(path, hash);
        }
    }
    let key = format!(
        "{:?}|{:?}|{}|{}|{}|{:?}",
        home,
        repo,
        rev,
        serde_json::to_string(a).map_err(err)?,
        serde_json::to_string(b).map_err(err)?,
        disk
    );
    if let Some(v) = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .get(&key)
        .cloned()
    {
        return Ok(v);
    }
    let x = surface(&store, &scope, a)?;
    let y = surface(&store, &scope, b)?;
    if x.state != "proven" || y.state != "proven" {
        return Ok(result(a, b, "graph stale".into()));
    }
    let mut reason = None;
    for (s, other) in [(&x, &y), (&y, &x)] {
        // A reference may be owned by the file node (for example, an import),
        // so inspect all resolved and ambiguous references, not just dependents.
        reason = store.read(|c| {
            let mut q = c.prepare("SELECT src.locator,target.key,r.candidates FROM g_refs r JOIN g_sources src ON src.id=r.source LEFT JOIN g_nodes target ON target.id=r.target WHERE src.scope=? AND r.resolution IN ('resolved','ambiguous') ORDER BY src.locator,r.id").map_err(err)?;
            let rows = q.query_map(params![scope.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?))).map_err(err)?;
            for row in rows {
                let (path, target, candidates) = row.map_err(err)?;
                if !other.files.contains(&path) { continue; }
                if let Some(target) = target.filter(|k| s.symbols.contains(k)) {
                    return Ok(Some(format!("symbol {target}")));
                }
                let v: serde_json::Value = serde_json::from_str(&candidates).map_err(err)?;
                if let Some(key) = v["keys"].as_array().and_then(|xs| xs.iter().filter_map(|x| x.as_str()).find(|k| s.symbols.contains(*k))) {
                    return Ok(Some(format!("symbol {key}")));
                }
            }
            Ok(None)
        })?;
        if reason.is_some() {
            break;
        }
        for h in &s.hotspots {
            let dir = h.rsplit_once('/').map(|x| x.0).unwrap_or("");
            if other
                .files
                .iter()
                .any(|p| dir.is_empty() || p.starts_with(&(dir.to_owned() + "/")))
            {
                reason = Some(format!("hotspot {h}"));
                break;
            }
        }
        if reason.is_some() {
            break;
        }
    }
    let out = reason.and_then(|r| result(a, b, r));
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(key, out.clone());
    Ok(out)
}
pub fn repo_for_store(store: &Path) -> Option<PathBuf> {
    let id = store.file_name()?.to_str()?;
    if !orch::is_wish_id(id) {
        return None;
    }
    let home = store.parent()?.parent()?.parent()?;
    for entry in fs::read_dir(home.join("goals")).ok()?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(session) = p.file_stem().and_then(|x| x.to_str()) else {
            continue;
        };
        if let Ok(Some(g)) = crate::goal::load(home, session) {
            if g.wish_id.as_deref() == Some(id) {
                return g.cwd.map(PathBuf::from);
            }
        }
    }
    None
}
pub fn blocking(
    store: &Path,
    candidate: &orch::Module,
    active: &[orch::Module],
) -> Res<Option<Conflict>> {
    let repo = repo_for_store(store);
    for other in active {
        if candidate.id == other.id {
            continue;
        }
        if let Some(repo) = &repo {
            let Some(home) = store.parent().and_then(Path::parent).and_then(Path::parent) else {
                continue;
            };
            if let Some(c) = conflict(home, repo, candidate, other)? {
                return Ok(Some(c));
            }
        } else if candidate.shared_owner.as_deref() != Some(&other.id)
            && other.shared_owner.as_deref() != Some(&candidate.id)
        {
            for x in &candidate.paths {
                for y in &other.paths {
                    if path_overlap(x, y) {
                        return Ok(result(candidate, other, format!("path {x}")));
                    }
                }
            }
        }
    }
    Ok(None)
}
