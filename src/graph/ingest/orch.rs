use super::*;
use crate::graph::{code, types::keys};
use crate::orch;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

fn git(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let o = crate::base::proc::cmd("git")
        .current_dir(repo)
        .args(args)
        .output()
        .ok()?;
    o.status.success().then_some(o.stdout)
}
fn tree(s: &str) -> &str {
    s.strip_prefix("tree:").unwrap_or(s)
}
fn path_ok(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.split('/').any(|s| s.is_empty() || s == "..")
        && !(p.len() > 1 && p.as_bytes()[1] == b':')
}
fn add_file(store: &GraphStore, b: &mut Batch, p: &str) -> Res<Option<String>> {
    if !path_ok(p) {
        return Ok(None);
    }
    let k = keys::file(p);
    if store
        .node(&NodeRef {
            scope: b.scope,
            key: k.clone(),
        })?
        .is_none()
        && !b.nodes.iter().any(|n| n.key == k)
    {
        b.nodes.push(node(k.clone(), "file", p, empty()));
    }
    Ok(Some(k))
}
fn indexed(store: &GraphStore, scope: i64) -> Res<Vec<(String, String, Value)>> {
    store.read(|c| {
        let mut q = c.prepare("SELECT key,kind,attrs FROM g_nodes n WHERE scope=? AND state='active' AND (kind='symbol' OR (kind='file' AND EXISTS (SELECT 1 FROM g_files f WHERE f.node=n.id)))").map_err(|e|Failure::new("io",e.to_string()))?;
        let rows = q.query_map([scope], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|Failure::new("io",e.to_string()))?.collect::<Result<Vec<_>,_>>().map_err(|e|Failure::new("io",e.to_string()))?;
        Ok(rows.into_iter().filter_map(|(k,t,a)| serde_json::from_str(&a).ok().map(|v| (k,t,v))).collect())
    })
}
fn spans_at(repo: &Path, commit: &str, path: &str) -> Vec<(String, u32, u32)> {
    let Some(language) = code::language_of(path) else {
        return vec![];
    };
    let spec = format!("{}:{path}", tree(commit));
    let Some(bytes) = git(repo, &["show", &spec]) else {
        return vec![];
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return vec![];
    };
    let extraction = code::extract(language, &text);
    let keys = code::symbol_keys(path, &extraction.defs);
    keys.into_iter()
        .zip(extraction.defs)
        .map(|(k, d)| (k, d.span.line, d.span.end_line))
        .collect()
}
fn report_paths(v: &Value, repo: &Path) -> Vec<String> {
    v.get("file_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("path").and_then(Value::as_str))
        .filter_map(|p| {
            let path = Path::new(p);
            let relative = if path.is_absolute() {
                path.strip_prefix(repo).ok()?
            } else {
                path
            };
            Some(relative.to_string_lossy().replace('\\', "/"))
        })
        .collect()
}
fn ranges(diff: &str) -> Vec<(u32, u32, u32, u32)> {
    diff.lines()
        .filter_map(|l| {
            let h = l.strip_prefix("@@ ")?.split(" @@").next()?;
            let (a, b) = h.split_once(' ')?;
            fn part(s: &str) -> Option<(u32, u32)> {
                let s = &s[1..];
                let (a, b) = s.split_once(',').unwrap_or((s, "1"));
                Some((a.parse().ok()?, b.parse().ok()?))
            }
            let (a, ac) = part(a)?;
            let (b, bc) = part(b)?;
            Some((a, ac, b, bc))
        })
        .collect()
}
fn overlaps(start: u32, end: u32, line: u32, count: u32) -> bool {
    count > 0 && start <= line.saturating_add(count - 1) && end >= line
}
fn save(
    store: &GraphStore,
    b: Batch,
    current: &mut HashSet<(String, String)>,
    report: &mut IngestReport,
) -> Res<()> {
    current.insert((b.source.locator.clone(), b.source.sha256.clone()));
    write(store, &b, report)
}

pub fn sync_orch(store: &GraphStore, wish_store: &Path, repo: &Path) -> Res<IngestReport> {
    let wish = wish_store
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Failure::new("invalid_argument", "invalid wish store"))?;
    let prefix = format!("orch:{wish}|");
    let scope = store.scope(repo, None)?.id;
    let units = orch::read_units(wish_store)?;
    let claims = orch::read_claims(wish_store)?;
    let map_path = wish_store.join("modules.md");
    let map_bytes = match std::fs::read(&map_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    let map = map_bytes
        .as_ref()
        .map(|b| {
            orch::parse_map(&String::from_utf8_lossy(b))
                .map_err(|e| Failure::new("invalid_argument", e))
        })
        .transpose()?;
    let mut result = IngestReport::default();
    let mut current = HashSet::new();
    let units_bytes = std::fs::read(wish_store.join("units.tsv"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    let mut b = batch(scope, "orch", format!("{prefix}units.tsv"), &units_bytes);
    b.nodes
        .push(node(keys::wish(wish), "wish", wish, json!({"wish":wish})));
    for u in &units {
        b.nodes.push(node(
            keys::unit(wish, &u.id),
            "unit",
            &u.id,
            serde_json::to_value(u).unwrap(),
        ));
    }
    save(store, b, &mut current, &mut result)?;
    let idx = indexed(store, scope)?;
    if let (Some(map), Some(bytes)) = (&map, &map_bytes) {
        let mut b = batch(scope, "orch", format!("{prefix}modules.md"), bytes);
        for m in &map.modules {
            b.nodes.push(node(
                keys::module(wish, &m.id),
                "module",
                &m.id,
                serde_json::to_value(m).unwrap(),
            ));
        }
        for m in &map.modules {
            let mk = keys::module(wish, &m.id);
            for dep in &m.depends {
                if map.modules.iter().any(|x| &x.id == dep) {
                    b.edges.push(edge(
                        scope,
                        &mk,
                        &keys::module(wish, dep),
                        "depends_on",
                        "",
                        empty(),
                    ));
                }
            }
            for (key, kind, _) in &idx {
                if kind == "file" && orch::covers(&m.paths, key.strip_prefix("file:").unwrap_or(""))
                {
                    b.edges.push(edge(scope, &mk, key, "owns", "", empty()));
                }
            }
        }
        for u in &units {
            if map.modules.iter().any(|m| m.id == u.track) {
                b.edges.push(edge(
                    scope,
                    &keys::unit(wish, &u.id),
                    &keys::module(wish, &u.track),
                    "in_module",
                    "",
                    empty(),
                ));
            }
        }
        save(store, b, &mut current, &mut result)?;
    }
    let claims_path = wish_store.join("claims.tsv");
    if claims_path.exists() {
        let bytes = std::fs::read(claims_path).map_err(|e| Failure::new("io", e.to_string()))?;
        let mut b = batch(scope, "orch", format!("{prefix}claims.tsv"), &bytes);
        let mut counts = HashMap::new();
        for c in claims {
            let n = counts.entry(c.unit.clone()).or_insert(0);
            *n += 1;
            let k = keys::scope_claim(wish, &c.unit, *n);
            b.nodes.push(node(k.clone(),"scope_claim",&c.unit,json!({"attempt":c.attempt,"attempt_number":*n,"paths":c.scope,"started":c.started,"module":c.module,"wish":c.wish,"pid":c.pid,"peers":c.peers})));
            for p in c.scope {
                if (p.ends_with('/') || p.contains('*'))
                    && !p.starts_with('/')
                    && !p.split('/').any(|part| part == "..")
                {
                    let target = format!(
                        "{}/{}",
                        keys::unit(wish, &c.unit).replacen("unit:", "source:orch:", 1),
                        hash(p.as_bytes())
                    );
                    if !b.nodes.iter().any(|node| node.key == target) {
                        b.nodes
                            .push(node(target.clone(), "source", &p, json!({"path": p})));
                    }
                    b.edges
                        .push(edge(scope, &k, &target, "claims_path", "", empty()));
                } else if let Some(f) = add_file(store, &mut b, &p)? {
                    b.edges
                        .push(edge(scope, &k, &f, "claims_path", "", empty()));
                }
            }
        }
        save(store, b, &mut current, &mut result)?;
    }
    for u in &units {
        let uk = keys::unit(wish, &u.id);
        let rows = orch::evidence(wish_store, &u.id)?;
        let waivers = orch::waivers(&rows);
        let ev_path = wish_store
            .join("reports")
            .join(format!("{}.evidence.tsv", u.id));
        let mut diff_files = HashSet::new();
        if let Ok(bytes) = std::fs::read(&ev_path) {
            let mut b = batch(
                scope,
                "orch",
                format!("{prefix}reports/{}.evidence.tsv", u.id),
                &bytes,
            );
            let base = rows
                .iter()
                .find(|r| r.kind == "run" && !r.commit.is_empty());
            let last = rows.iter().rev().find(|r| !r.commit.is_empty());
            if let (Some(a), Some(z)) = (base, last) {
                if let Some(files) = git(
                    repo,
                    &[
                        "diff",
                        "--name-only",
                        "-z",
                        tree(&a.commit),
                        tree(&z.commit),
                        "--",
                    ],
                ) {
                    for raw in files.split(|x| *x == 0).filter(|x| !x.is_empty()) {
                        let Ok(p) = std::str::from_utf8(raw) else {
                            continue;
                        };
                        if let Some(f) = add_file(store, &mut b, p)? {
                            diff_files.insert(p.to_string());
                            b.edges
                                .push(edge(scope, &uk, &f, "changes", "diff", empty()));
                            if let Some(d) = git(
                                repo,
                                &[
                                    "diff",
                                    "--no-ext-diff",
                                    "--unified=0",
                                    tree(&a.commit),
                                    tree(&z.commit),
                                    "--",
                                    p,
                                ],
                            ) {
                                let hunks = ranges(&String::from_utf8_lossy(&d));
                                for (commit, old) in [(&a.commit, true), (&z.commit, false)] {
                                    for (key, start, end) in spans_at(repo, commit, p) {
                                        if !idx.iter().any(|(k, t, _)| k == &key && t == "symbol") {
                                            continue;
                                        }
                                        if hunks.iter().any(|(al, ac, bl, bc)| {
                                            if old {
                                                overlaps(start, end, *al, *ac)
                                            } else {
                                                overlaps(start, end, *bl, *bc)
                                            }
                                        }) && !b
                                            .edges
                                            .iter()
                                            .any(|e| e.dst.key == key && e.kind == "changes")
                                        {
                                            b.edges.push(edge(
                                                scope,
                                                &uk,
                                                &key,
                                                "changes",
                                                "diff",
                                                empty(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            save(store, b, &mut current, &mut result)?;
        }
        for suffix in ["json", "md", "review.md", "verdict.json"] {
            let name = format!("reports/{}.{}", u.id, suffix);
            let path = wish_store.join(&name);
            let bytes = match std::fs::read(&path) {
                Ok(x) => x,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(Failure::new("io", e.to_string())),
            };
            let mut b = batch(scope, "orch", format!("{prefix}{name}"), &bytes);
            let ak = keys::artifact(&format!("orch/{wish}/{name}"));
            b.nodes.push(node(
                ak,
                "artifact",
                &name,
                json!({"sha256":b.source.sha256}),
            ));
            if suffix == "json" {
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    for p in report_paths(&v, repo) {
                        if diff_files.contains(&p) {
                            continue;
                        }
                        if let Some(f) = add_file(store, &mut b, &p)? {
                            b.edges
                                .push(edge(scope, &uk, &f, "changes", "report", empty()));
                        }
                    }
                }
            }
            if suffix == "verdict.json" {
                if let Some(v) = orch::read_verdict(wish_store, &u.id)? {
                    for f in v.findings {
                        let mut sha = String::new();
                        let loc = f
                            .location
                            .rsplit_once(':')
                            .and_then(|(p, n)| n.parse::<u32>().ok().map(|n| (p, n)));
                        if let Some((p, _)) = loc {
                            if path_ok(p) {
                                let spec = format!("{}:{p}", tree(&v.tree));
                                if let Some(content) = git(repo, &["show", &spec]) {
                                    sha = hash(&content)
                                };
                            }
                        }
                        let original = keys::finding(wish, &u.id, f.number as u32);
                        let fk = match store.node(&NodeRef {
                            scope,
                            key: original.clone(),
                        })? {
                            Some(existing)
                                if existing.attrs["tree"] != v.tree
                                    || existing.attrs["sha256"] != sha =>
                            {
                                format!(
                                    "{original}@{}",
                                    hash(format!("{}\0{}", v.tree, sha).as_bytes())
                                )
                            }
                            _ => original,
                        };
                        if let Some((p, line)) = loc {
                            if let Some(file) = add_file(store, &mut b, p)? {
                                b.edges
                                    .push(edge(scope, &fk, &file, "located_in", "", empty()));
                                for (key, start, end) in spans_at(repo, &v.tree, p) {
                                    if start <= line
                                        && line <= end
                                        && idx.iter().any(|(k, t, _)| k == &key && t == "symbol")
                                    {
                                        b.edges.push(edge(scope, &fk, &key, "about", "", empty()));
                                    }
                                }
                            }
                        }
                        let mut attrs = json!({"number":f.number,"severity":format!("{:?}",f.severity).to_lowercase(),"location":f.location,"tree":v.tree,"sha256":sha,"evidence":f.evidence,"check":f.check,"fix":f.fix});
                        if waivers
                            .iter()
                            .any(|w| w.number == f.number && w.tree == v.tree)
                        {
                            attrs["waived"] = json!(true);
                        }
                        b.nodes.push(node(
                            fk,
                            "finding",
                            &format!("{} #{}", u.id, f.number),
                            attrs,
                        ));
                    }
                }
            }
            save(store, b, &mut current, &mut result)?;
        }
    }
    let modules: Vec<Option<&str>> = std::iter::once(None)
        .chain(
            map.as_ref()
                .into_iter()
                .flat_map(|m| m.modules.iter().map(|x| Some(x.id.as_str()))),
        )
        .collect();
    for m in modules {
        if let Some(h) = orch::read_handoff(wish_store, m)? {
            let path = orch::handoff_path(wish_store, m);
            let bytes = std::fs::read(&path).map_err(|e| Failure::new("io", e.to_string()))?;
            let name = if let Some(m) = m {
                format!("handoffs/module/{m}.json")
            } else {
                "handoffs/wish.json".into()
            };
            let mut b = batch(scope, "orch", format!("{prefix}{name}"), &bytes);
            if !h.decisions.trim().is_empty() {
                for u in units.iter().filter(|u| {
                    m.map_or_else(
                        || {
                            orch::read_handoff(wish_store, Some(&u.track))
                                .ok()
                                .flatten()
                                .is_none()
                        },
                        |m| u.track == m,
                    )
                }) {
                    let k = keys::decision(wish, &u.id, 1);
                    b.nodes.push(node(
                        k.clone(),
                        "decision",
                        &h.decisions,
                        json!({"text":h.decisions,"timestamp":h.ts}),
                    ));
                    b.edges.push(edge(
                        scope,
                        &k,
                        &keys::unit(wish, &u.id),
                        "decided_in",
                        "",
                        empty(),
                    ));
                }
            }
            save(store, b, &mut current, &mut result)?;
        }
    }
    stale(store, scope, "orch", &prefix, &current, &mut result)?;
    result.revision = store.revision()?;
    Ok(result)
}
