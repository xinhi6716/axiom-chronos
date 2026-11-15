use super::{
    index::{ArtifactStatus, Index},
    manifest::{GroupBy, Role},
    read,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ReproStatus {
    Pass,
    Fail { overlap: Vec<String> },
    NotApplicable { reason: String },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReproResult {
    pub status: ReproStatus,
    pub pilot_groups: usize,
    pub acceptance_groups: usize,
    pub pilot_sha256: Option<String>,
    pub acceptance_sha256: Option<String>,
    pub rule: String,
}

fn root(parent: &mut BTreeMap<String, String>, id: &str) -> String {
    parent.entry(id.to_owned()).or_insert_with(|| id.to_owned());
    let next = parent[id].clone();
    if next == id {
        return next;
    }
    let r = root(parent, &next);
    parent.insert(id.to_owned(), r.clone());
    r
}
fn union(parent: &mut BTreeMap<String, String>, a: &str, b: &str) {
    let a = root(parent, a);
    let b = root(parent, b);
    if a < b {
        parent.insert(b, a);
    } else if b < a {
        parent.insert(a, b);
    }
}
pub fn study_groups(ix: &Index) -> BTreeMap<String, String> {
    let mut parent = BTreeMap::new();
    let mut duplicates = BTreeMap::<String, String>::new();
    for a in ix.located().by_role(Role::Inventory) {
        for i in 0..ix.len(&a.id) {
            let Some(row) = ix.row(&a.id, i as u32) else {
                continue;
            };
            let id = row.get("paper_id").trim();
            if id.is_empty() {
                continue;
            }
            root(&mut parent, id);
            let dup = row.get("duplicate_group").trim();
            if !dup.is_empty() {
                if let Some(other) = duplicates.get(dup) {
                    union(&mut parent, id, other);
                } else {
                    duplicates.insert(dup.into(), id.into());
                }
            }
            for col in ["supplement_of", "erratum_of"] {
                let other = row.get(col).trim();
                if !other.is_empty() {
                    union(&mut parent, id, other);
                }
            }
        }
    }
    if let Some(groups) = &ix.located().manifest.groups {
        for id in [&groups.pilot, &groups.acceptance] {
            for i in 0..ix.len(id) {
                if let Some(row) = ix.row(id, i as u32) {
                    let paper = row.get("paper_id").trim();
                    if !paper.is_empty() {
                        root(&mut parent, paper);
                    }
                }
            }
        }
    }
    let keys: Vec<_> = parent.keys().cloned().collect();
    keys.into_iter()
        .map(|id| {
            let r = root(&mut parent, &id);
            (id, r)
        })
        .collect()
}
pub fn guard(ix: &Index) -> ReproResult {
    let l = ix.located();
    let Some(g) = &l.manifest.groups else {
        return ReproResult {
            status: ReproStatus::NotApplicable {
                reason: "groups not declared".into(),
            },
            pilot_groups: 0,
            acceptance_groups: 0,
            pilot_sha256: None,
            acceptance_sha256: None,
            rule: String::new(),
        };
    };
    let rule = match &g.by {
        GroupBy::Column(c) => format!("column:{c}"),
        GroupBy::StudyGroups => "study_groups".into(),
    };
    let mut result = ReproResult {
        status: ReproStatus::NotApplicable {
            reason: String::new(),
        },
        pilot_groups: 0,
        acceptance_groups: 0,
        pilot_sha256: None,
        acceptance_sha256: None,
        rule,
    };
    for id in [&g.pilot, &g.acceptance] {
        if !matches!(ix.status(id), ArtifactStatus::Ready(_)) || read::verify(l, id).is_err() {
            result.status = ReproStatus::NotApplicable {
                reason: format!("artifact {id} missing or corrupt"),
            };
            return result;
        }
    }
    if matches!(g.by, GroupBy::StudyGroups) {
        if l.by_role(Role::Inventory)
            .iter()
            .any(|a| !matches!(ix.status(&a.id), ArtifactStatus::Ready(_)))
        {
            result.status = ReproStatus::NotApplicable {
                reason: "inventory missing or corrupt".into(),
            };
            return result;
        }
    }
    result.pilot_sha256 = read::verify(l, &g.pilot).ok();
    result.acceptance_sha256 = read::verify(l, &g.acceptance).ok();
    let mapping = study_groups(ix);
    let collect = |id: &str| -> BTreeSet<String> {
        (0..ix.len(id))
            .filter_map(|i| ix.row(id, i as u32))
            .filter_map(|r| {
                let v = match &g.by {
                    GroupBy::Column(c) => r.get(c).trim(),
                    GroupBy::StudyGroups => r.get("paper_id").trim(),
                };
                if v.is_empty() {
                    None
                } else {
                    Some(match &g.by {
                        GroupBy::Column(_) => v.to_owned(),
                        GroupBy::StudyGroups => {
                            mapping.get(v).cloned().unwrap_or_else(|| v.to_owned())
                        }
                    })
                }
            })
            .collect()
    };
    let p = collect(&g.pilot);
    let a = collect(&g.acceptance);
    result.pilot_groups = p.len();
    result.acceptance_groups = a.len();
    let overlap: Vec<_> = p.intersection(&a).cloned().collect();
    result.status = if overlap.is_empty() {
        ReproStatus::Pass
    } else {
        ReproStatus::Fail { overlap }
    };
    result
}
