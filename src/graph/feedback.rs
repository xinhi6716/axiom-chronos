//! Collect explicit decisions as project-wide feedback memories.
use super::{keys, Batch, EdgeSpec, GraphStore, IngestReport, NodeRef, NodeSpec, SourceSpec};
use crate::{
    base::{Failure, Res},
    council::Council,
    goal::Goal,
    orch,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FeedbackEvent {
    pub kind: String,
    pub target: String,
    pub decider: String,
    pub reason: String,
    pub text: String,
    pub wish: String,
    pub source: String,
    pub source_sha256: String,
    pub supersedes: Option<String>,
}

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn locator(home: &Path, path: &Path) -> String {
    path.strip_prefix(home)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
fn entries(dir: &Path) -> Res<Vec<PathBuf>> {
    let iter = match fs::read_dir(dir) {
        Ok(iter) => iter,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(io(e)),
    };
    let mut paths = iter
        .map(|e| e.map(|x| x.path()).map_err(io))
        .collect::<Res<Vec<_>>>()?;
    paths.sort();
    Ok(paths)
}
fn councils(
    home: &Path,
    dir: &Path,
    depth: usize,
    wish: &str,
    out: &mut Vec<FeedbackEvent>,
) -> Res<()> {
    for path in entries(dir)? {
        let ty = fs::symlink_metadata(&path).map_err(io)?.file_type();
        if ty.is_dir() && depth < 3 {
            councils(home, &path, depth + 1, wish, out)?;
        }
        if !ty.is_file() || path.file_name().and_then(|s| s.to_str()) != Some("council.json") {
            continue;
        }
        let bytes = fs::read(&path).map_err(io)?;
        let council: Council = serde_json::from_slice(&bytes)
            .map_err(|e| Failure::new("invalid_json", format!("{}: {e}", path.display())))?;
        for (i, claim) in council.claims.into_iter().enumerate() {
            if let Some(ruling) = claim
                .ruling
                .filter(|r| r.decision == "rejected" || r.decision == "narrowed")
            {
                let source = format!("{}#claims/{i}", locator(home, &path));
                out.push(FeedbackEvent {
                    kind: "overruled_ruling".into(),
                    target: source.clone(),
                    decider: "core".into(),
                    reason: ruling.reason,
                    text: claim.text,
                    wish: wish.into(),
                    source,
                    source_sha256: hash(&bytes),
                    supersedes: None,
                });
            }
        }
    }
    Ok(())
}

pub fn collect(home: &Path, project: &str) -> Res<Vec<FeedbackEvent>> {
    let project_store = orch::wish_store(home, project, None)?;
    let mut out = Vec::new();
    for path in entries(&home.join("goals"))? {
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).map_err(io)?;
        let goal: Goal = serde_json::from_slice(&bytes)
            .map_err(|e| Failure::new("invalid_json", format!("{}: {e}", path.display())))?;
        if goal.project.as_deref() != Some(project) {
            continue;
        }
        let wish = goal.wish_id.unwrap_or_default();
        let target = keys::wish(&wish);
        let supersedes = goal
            .other
            .get("supersedes")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let digest = hash(&bytes);
        for (i, text) in goal.amendments.into_iter().enumerate() {
            out.push(FeedbackEvent {
                kind: "correction".into(),
                target: target.clone(),
                decider: "user".into(),
                reason: String::new(),
                text,
                wish: wish.clone(),
                source: format!("{}#amendments/{i}", locator(home, &path)),
                source_sha256: digest.clone(),
                supersedes: supersedes.clone(),
            });
        }
        if goal.reason.as_deref() == Some("user_stop") {
            let note = goal.note.unwrap_or_default();
            out.push(FeedbackEvent {
                kind: "user_stop".into(),
                target,
                decider: "user".into(),
                reason: note.clone(),
                text: note,
                wish,
                source: format!("{}#reason", locator(home, &path)),
                source_sha256: digest,
                supersedes,
            });
        }
    }
    for store in orch::wish_stores(&project_store) {
        let wish = orch::wish_of_store(&store);
        councils(home, &store, 0, &wish, &mut out)?;
        if !store.join("units.tsv").exists() {
            continue;
        }
        for unit in orch::read_units(&store)? {
            let rows = orch::evidence(&store, &unit.id)?;
            let waivers = orch::findings::waivers(&rows);
            if waivers.is_empty() {
                continue;
            }
            let Some(verdict) = orch::findings::read_verdict(&store, &unit.id)? else {
                continue;
            };
            let path = store
                .join("reports")
                .join(format!("{}.evidence.tsv", unit.id));
            let digest = hash(&fs::read(&path).map_err(io)?);
            for (i, waiver) in waivers.into_iter().enumerate() {
                let Some(finding) = verdict.findings.iter().find(|f| f.number == waiver.number)
                else {
                    continue;
                };
                let Ok(number) = u32::try_from(finding.number) else {
                    continue;
                };
                out.push(FeedbackEvent {
                    kind: "waived_finding".into(),
                    target: keys::finding(&wish, &unit.id, number),
                    decider: "core".into(),
                    reason: waiver.reason,
                    text: finding.evidence.clone(),
                    wish: wish.clone(),
                    source: format!("{}#waivers/{i}", locator(home, &path)),
                    source_sha256: digest.clone(),
                    supersedes: None,
                });
            }
        }
    }
    out.sort_by(|a, b| (&a.wish, &a.source, &a.kind).cmp(&(&b.wish, &b.source, &b.kind)));
    Ok(out)
}

pub fn ingest(
    store: &GraphStore,
    home: &Path,
    project: &str,
    project_dir: &Path,
) -> Res<IngestReport> {
    let events = collect(home, project)?;
    let scope = store.scope(project_dir, None)?;
    let mut report = IngestReport::default();
    for event in events {
        let key =
            keys::feedback(&hash(format!("{}\n{}", event.source, event.text).as_bytes())[..16]);
        let node = NodeSpec {
            key: key.clone(),
            kind: "feedback".into(),
            label: event.text.chars().take(80).collect(),
            summary: event.text.clone(),
            attrs: serde_json::to_value(&event).map_err(io)?,
            entry_id: None,
            state: "active".into(),
        };
        let mut edges = Vec::new();
        if let Some(target) = &event.supersedes {
            let dst = NodeRef {
                scope: scope.id,
                key: target.clone(),
            };
            if store.node(&dst)?.is_some() {
                edges.push(EdgeSpec {
                    src: NodeRef {
                        scope: scope.id,
                        key,
                    },
                    dst,
                    kind: "supersedes".into(),
                    locator: String::new(),
                    weight: 500,
                    state: "active".into(),
                    attrs: serde_json::json!({}),
                });
            }
        }
        let write = store.write(&Batch {
            scope: scope.id,
            source: SourceSpec {
                kind: "feedback".into(),
                locator: event.source.clone(),
                sha256: event.source_sha256.clone(),
                version: "1".into(),
                state: "active".into(),
            },
            nodes: vec![node],
            edges,
            replace_source: false,
        })?;
        report.nodes_written += write.nodes_inserted + write.nodes_updated;
        report.edges_written += write.edges_inserted;
        if write.nodes_inserted + write.nodes_updated == 0 {
            report.unchanged += 1;
        }
    }
    report.revision = store.revision()?;
    Ok(report)
}
