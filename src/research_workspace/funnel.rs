use super::{
    index::{ArtifactStatus, Index, RowRef},
    manifest::{Capability, Role},
};
use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Total,
    Deduplicated,
    Duplicates,
    ScreenedIn,
    ScreenedOut { reason: Option<String> },
    Pending,
    Unprocessed,
    Read,
    ReadFailed { reason: Option<String> },
}
impl Node {
    pub fn key(&self) -> String {
        match self {
            Self::Total => "total".into(),
            Self::Deduplicated => "deduplicated".into(),
            Self::Duplicates => "duplicates".into(),
            Self::ScreenedIn => "screened_in".into(),
            Self::ScreenedOut { reason: None } => "screened_out".into(),
            Self::ScreenedOut { reason: Some(r) } => format!("screened_out:{r}"),
            Self::Pending => "pending".into(),
            Self::Unprocessed => "unprocessed".into(),
            Self::Read => "read".into(),
            Self::ReadFailed { reason: None } => "read_failed".into(),
            Self::ReadFailed { reason: Some(r) } => format!("read_failed:{r}"),
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "total" => Self::Total,
            "deduplicated" => Self::Deduplicated,
            "duplicates" => Self::Duplicates,
            "screened_in" => Self::ScreenedIn,
            "screened_out" => Self::ScreenedOut { reason: None },
            "pending" => Self::Pending,
            "unprocessed" => Self::Unprocessed,
            "read" => Self::Read,
            "read_failed" => Self::ReadFailed { reason: None },
            _ => {
                if let Some(r) = key.strip_prefix("screened_out:") {
                    Self::ScreenedOut {
                        reason: Some(r.into()),
                    }
                } else if let Some(r) = key.strip_prefix("read_failed:") {
                    Self::ReadFailed {
                        reason: Some(r.into()),
                    }
                } else {
                    return None;
                }
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Decision {
    Included,
    Excluded { reason: String },
    Pending { reason: String },
    Failed { reason: String },
    Duplicate { of_group: String },
    Unprocessed,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunnelRow {
    pub record_id: String,
    pub path: String,
    pub sha256: String,
    pub doi: String,
    pub title: String,
    pub duplicate_group: String,
    pub decision: Decision,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Funnel {
    pub total: usize,
    pub deduplicated: usize,
    pub duplicates: usize,
    pub screened_in: usize,
    pub screened_out: usize,
    pub out_reasons: Vec<(String, usize)>,
    pub pending: usize,
    pub unprocessed: usize,
    pub read: usize,
    pub read_failed: usize,
    pub fail_reasons: Vec<(String, usize)>,
}

// An absent screening artifact means no decisions; an unavailable declared
// artifact must not silently produce misleading counts.
fn scan(ix: &Index, mut visit: impl FnMut(RowRef<'_>, Decision)) -> Res<()> {
    let located = ix.located();
    located.require(Capability::Literature)?;
    let inventory = located
        .by_role(Role::Inventory)
        .into_iter()
        .next()
        .ok_or_else(|| Failure::new("missing_artifact", "inventory missing"))?;
    let screening = located.by_role(Role::Screening).into_iter().next();
    for artifact in std::iter::once(inventory).chain(screening) {
        let status = ix.status(&artifact.id);
        if !matches!(status, ArtifactStatus::Ready(_)) {
            return Err(Failure::new(
                "unavailable",
                format!("{}: {status:?}", artifact.id),
            ));
        }
    }
    for i in 0..ix.len(&inventory.id) {
        // Index::build rejects tables whose row count exceeds u32.
        let row = ix
            .row(&inventory.id, i as u32)
            .expect("indexed inventory row");
        let decision = if row.get("canonical") != "1" {
            Decision::Duplicate {
                of_group: row.get("duplicate_group").into(),
            }
        } else if !row.get("read_error").is_empty() {
            Decision::Failed {
                reason: row.get("read_error").into(),
            }
        } else {
            let screened = screening.and_then(|artifact| {
                ix.find(&artifact.id, row.get("sha256"))
                    .and_then(|i| ix.row(&artifact.id, i))
            });
            match screened {
                Some(s) => match s.get("verdict") {
                    "include" => Decision::Included,
                    "exclude" => Decision::Excluded {
                        reason: s.get("reason").into(),
                    },
                    "pending" => Decision::Pending {
                        reason: s.get("pending_reason").into(),
                    },
                    _ => Decision::Unprocessed,
                },
                None => Decision::Unprocessed,
            }
        };
        visit(row, decision);
    }
    Ok(())
}

fn matches_node(node: &Node, decision: &Decision) -> bool {
    match node {
        Node::Total => true,
        Node::Deduplicated => !matches!(decision, Decision::Duplicate { .. }),
        Node::Duplicates => matches!(decision, Decision::Duplicate { .. }),
        Node::ScreenedIn | Node::Read => matches!(decision, Decision::Included),
        Node::ScreenedOut { reason } => match decision {
            Decision::Excluded { reason: actual } => reason.as_ref().map_or(true, |r| r == actual),
            _ => false,
        },
        Node::Pending => matches!(decision, Decision::Pending { .. }),
        Node::Unprocessed => matches!(decision, Decision::Unprocessed),
        Node::ReadFailed { reason } => match decision {
            Decision::Failed { reason: actual } => reason.as_ref().map_or(true, |r| r == actual),
            _ => false,
        },
    }
}
fn sorted_reasons(reasons: HashMap<String, usize>) -> Vec<(String, usize)> {
    let mut reasons: Vec<_> = reasons.into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    reasons
}
pub fn funnel(ix: &Index) -> Res<Funnel> {
    let mut f = Funnel {
        total: 0,
        deduplicated: 0,
        duplicates: 0,
        screened_in: 0,
        screened_out: 0,
        out_reasons: vec![],
        pending: 0,
        unprocessed: 0,
        read: 0,
        read_failed: 0,
        fail_reasons: vec![],
    };
    let mut out = HashMap::new();
    let mut failed = HashMap::new();
    scan(ix, |_, decision| {
        f.total += 1;
        if matches!(decision, Decision::Duplicate { .. }) {
            f.duplicates += 1;
        } else {
            f.deduplicated += 1;
        }
        match decision {
            Decision::Included => {
                f.screened_in += 1;
                f.read += 1;
            }
            Decision::Excluded { reason } => {
                f.screened_out += 1;
                *out.entry(if reason.is_empty() {
                    "(none)".into()
                } else {
                    reason
                })
                .or_insert(0) += 1;
            }
            Decision::Pending { .. } => f.pending += 1,
            Decision::Unprocessed => f.unprocessed += 1,
            Decision::Failed { reason } => {
                f.read_failed += 1;
                *failed
                    .entry(if reason.is_empty() {
                        "(none)".into()
                    } else {
                        reason
                    })
                    .or_insert(0) += 1;
            }
            Decision::Duplicate { .. } => {}
        }
    })?;
    f.out_reasons = sorted_reasons(out);
    f.fail_reasons = sorted_reasons(failed);
    Ok(f)
}
pub fn rows(ix: &Index, n: &Node, offset: usize, limit: usize) -> Res<(Vec<FunnelRow>, usize)> {
    let mut result = Vec::new();
    let mut total = 0;
    scan(ix, |row, decision| {
        if matches_node(n, &decision) {
            if total >= offset && result.len() < limit {
                result.push(FunnelRow {
                    record_id: row.record_id(),
                    path: row.get("path").into(),
                    sha256: row.get("sha256").into(),
                    doi: row.get("doi").into(),
                    title: row.get("title").into(),
                    duplicate_group: row.get("duplicate_group").into(),
                    decision,
                });
            }
            total += 1;
        }
    })?;
    Ok((result, total))
}
pub fn sample(ix: &Index, n: &Node, count: usize, seed: u64) -> Res<Vec<String>> {
    let mut ids = Vec::new();
    scan(ix, |row, decision| {
        if matches_node(n, &decision) {
            ids.push(row.record_id());
        }
    })?;
    // SplitMix64 and Fisher-Yates give a stable shuffle without a new dependency.
    let mut state = seed;
    for i in (1..ids.len()).rev() {
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = state;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^= x >> 31;
        ids.swap(i, (x % (i as u64 + 1)) as usize);
    }
    ids.truncate(count);
    Ok(ids)
}
