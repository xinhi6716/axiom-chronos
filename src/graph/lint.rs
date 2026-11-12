use super::{
    store::{kind_pair, project_key, GraphStore},
    types::{TYPED_MAX_BYTES, TYPED_MAX_LINKS, TYPED_MAX_NODES},
};
use crate::{
    base::{Failure, Res},
    memory::lint::Problem,
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
pub fn check(store: &GraphStore, project_dir: &Path) -> Res<Vec<Problem>> {
    let key = project_key(project_dir);
    store.read(|db| {let mut out=Vec::new();
 let mut s=db.prepare("SELECT e.src,e.dst,e.kind,a.kind,b.kind,sa.project_key,sb.project_key,ss.project_key FROM g_edges e LEFT JOIN g_nodes a ON a.id=e.src LEFT JOIN g_nodes b ON b.id=e.dst LEFT JOIN g_scopes sa ON sa.id=a.scope LEFT JOIN g_scopes sb ON sb.id=b.scope LEFT JOIN g_sources source ON source.id=e.source LEFT JOIN g_scopes ss ON ss.id=source.scope WHERE sa.project_key=?1 OR sb.project_key=?1 OR ss.project_key=?1").map_err(err)?;
 let mut rows=s.query([&key]).map_err(err)?;while let Some(r)=rows.next().map_err(err)? {let a:i64=r.get(0).map_err(err)?;let b:i64=r.get(1).map_err(err)?;let k:String=r.get(2).map_err(err)?;let ak:Option<String>=r.get(3).map_err(err)?;let bk:Option<String>=r.get(4).map_err(err)?;let ap:Option<String>=r.get(5).map_err(err)?;let bp:Option<String>=r.get(6).map_err(err)?;if ak.is_none()||bk.is_none(){out.push(Problem{kind:"missing_endpoint",message:format!("edge {a}->{b} has missing endpoint")});}else if !kind_pair(&k,ak.as_deref().unwrap(),bk.as_deref().unwrap()){out.push(Problem{kind:"bad_kind_pair",message:format!("edge {a}->{b} has invalid kind pair")});}if ap.is_some()&&bp.is_some()&&ap!=bp {out.push(Problem{kind:"cross_project",message:format!("edge {a}->{b} crosses projects")});}}
 let mut s=db.prepare("SELECT a.old_id,a.new_id FROM g_aliases a WHERE EXISTS(SELECT 1 FROM g_scopes WHERE project_key=?)").map_err(err)?;let pairs:HashMap<i64,i64>=s.query_map([&key],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;let starts:Vec<i64>={let mut st=db.prepare("SELECT n.id FROM g_nodes n JOIN g_scopes s ON s.id=n.scope WHERE s.project_key=?").map_err(err)?;let v=st.query_map([&key],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;v};let mut reported=HashSet::new();for &start in &starts{let mut path=HashSet::new();let mut n=start;while let Some(&next)=pairs.get(&n){if !path.insert(n){if reported.insert(n){out.push(Problem{kind:"alias_cycle",message:format!("alias cycle at {n}")});}break}n=next;}}
 let mut s=db.prepare("SELECT s.id,s.state,s.locator,s.kind FROM g_sources s JOIN g_scopes sc ON sc.id=s.scope WHERE sc.project_key=?").map_err(err)?;let mut rows=s.query([&key]).map_err(err)?;while let Some(r)=rows.next().map_err(err)? {let id:i64=r.get(0).map_err(err)?;let state:String=r.get(1).map_err(err)?;let locator:String=r.get(2).map_err(err)?;let kind:String=r.get(3).map_err(err)?;if state=="stale" {out.push(Problem{kind:"stale_source",message:format!("source {id} is stale")});}if state=="retired" || state=="tombstone" || (kind=="code" && !locator.is_empty() && !project_dir.join(&locator).exists()) {out.push(Problem{kind:"removed_source",message:format!("source {id} is removed")});}if kind=="typed" {let nodes:i64=db.query_row("SELECT count(*) FROM g_nodes WHERE source=?",[id],|r|r.get(0)).map_err(err)?;let edges:i64=db.query_row("SELECT count(*) FROM g_edges WHERE source=?",[id],|r|r.get(0)).map_err(err)?;let bytes:i64=db.query_row("SELECT COALESCE(sum(length(CAST(attrs AS BLOB))+length(CAST(label AS BLOB))+length(CAST(summary AS BLOB))),0) FROM g_nodes WHERE source=?",[id],|r|r.get(0)).map_err(err)?;if nodes>TYPED_MAX_NODES as i64||edges>TYPED_MAX_LINKS as i64||bytes>TYPED_MAX_BYTES as i64 {out.push(Problem{kind:"over_limit",message:format!("typed source {id} exceeds limits")});}}}
 Ok(out)})
}
