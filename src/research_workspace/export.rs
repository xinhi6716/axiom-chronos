use super::{
    annotations::{self, AnnotationStore, SnapshotEntry},
    dataset::{self, Cell, Conversion, DataLayer, DatasetQuery},
    evidence::{self, Locator, Target},
    index::{ArtifactStatus, Index},
    manifest::{Capability, Located, Role},
    read::{self, Limits},
    repro,
    seams::{SnapshotConsumer, SnapshotRef, VaultActions, VaultLink},
};
use crate::{
    base::{Failure, Res},
    seams::{sha256_hex, Clock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exported {
    pub path: PathBuf,
    pub rows: usize,
    pub sha256: String,
}

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn canonical_future(path: &Path) -> Res<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(io)?.join(path)
    };
    let mut clean = PathBuf::new();
    for c in absolute.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            _ => clean.push(c.as_os_str()),
        }
    }
    let mut ancestor = clean.as_path();
    let mut tail = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tail.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| io("invalid destination"))?
                        .to_os_string(),
                );
                ancestor = ancestor.parent().ok_or_else(|| io("invalid destination"))?;
            }
            Err(e) => return Err(io(e)),
        }
    }
    let mut result = fs::canonicalize(ancestor).map_err(io)?;
    for part in tail.iter().rev() {
        result.push(part);
    }
    Ok(result)
}
pub fn check_dest(l: &Located, dest: &Path) -> Res<PathBuf> {
    let dest = canonical_future(dest)?;
    for root in l.roots() {
        let root = canonical_future(&root)?;
        if dest.starts_with(&root) {
            return Err(Failure::new(
                "dest_in_source",
                format!("{} is inside {}", dest.display(), root.display()),
            ));
        }
    }
    Ok(dest)
}
static NEXT: AtomicU64 = AtomicU64::new(0);
fn temp_path(dir: &Path) -> PathBuf {
    dir.join(format!(
        ".research-export-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn rename_file(from: &Path, to: &Path) -> Res<()> {
    fs::rename(from, to).map_err(io)
}
fn atomic_stream<T>(
    dir: &Path,
    name: &str,
    write: impl FnOnce(&mut BufWriter<File>) -> Res<T>,
) -> Res<(T, PathBuf, String)> {
    let temp = temp_path(dir);
    let result = (|| {
        let f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(io)?;
        let mut f = BufWriter::new(f);
        let result = write(&mut f)?;
        f.flush().map_err(io)?;
        drop(f);
        let hash = read::hash_file(&temp)?;
        let path = dir.join(name);
        rename_file(&temp, &path)?;
        Ok((result, path, hash))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && !name.contains("..")
        && !name.contains(['/', '\\', ':', '\0'])
}
fn csv_field(f: &mut impl Write, s: &str) -> Res<()> {
    if s.contains([',', '"', '\r', '\n']) {
        write!(f, "\"{}\"", s.replace('"', "\"\"")).map_err(io)
    } else {
        f.write_all(s.as_bytes()).map_err(io)
    }
}
fn cell(c: &Cell) -> &str {
    match c {
        Cell::Value(v) => v,
        Cell::Empty { .. } => "",
    }
}
pub fn export_csv(ix: &Index, q: &DatasetQuery, dest_dir: &Path, file_name: &str) -> Res<Exported> {
    let dir = check_dest(ix.located(), dest_dir)?;
    if !valid_name(file_name) {
        return Err(Failure::new("invalid_argument", "invalid file name"));
    }
    check_dest(ix.located(), &dir.join(file_name))?;
    fs::create_dir_all(&dir).map_err(io)?;
    let (rows, path, sha256) = atomic_stream(&dir, file_name, |f| {
        f.write_all(b"record_id,layer,paper_or_source,condition_id,variable,entity,original,unit_original,canonical,unit_canonical,conversion,quality_grade,evidence_ids,flags,locator\r\n").map_err(io)?;
        let mut query = q.clone();
        query.offset = 0;
        query.limit = 500;
        let mut count = 0;
        loop {
            let page = dataset::page(ix, &query)?;
            for row in &page.rows {
                let conversion = match &row.conversion {
                    Conversion::Converted { rule } => rule.as_str(),
                    Conversion::NotConvertible { reason } => reason.as_str(),
                    Conversion::NotNumeric => "not_numeric",
                    Conversion::Same => "same",
                };
                let canonical = if matches!(row.conversion, Conversion::NotConvertible { .. }) {
                    ""
                } else {
                    cell(&row.canonical)
                };
                let evidence = row.evidence.join(";");
                let flags = row.flags.join(";");
                let fields = [
                    row.record_id.as_str(),
                    match row.layer {
                        DataLayer::Extracted => "extracted",
                        DataLayer::Raw => "raw",
                    },
                    &row.paper_or_source,
                    &row.condition_id,
                    &row.variable,
                    &row.entity,
                    cell(&row.original),
                    &row.unit_original,
                    canonical,
                    &row.unit_canonical,
                    conversion,
                    row.quality_grade.as_deref().unwrap_or(""),
                    &evidence,
                    &flags,
                    row.locator.as_deref().unwrap_or(""),
                ];
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_all(b",").map_err(io)?;
                    }
                    csv_field(f, field)?;
                }
                f.write_all(b"\r\n").map_err(io)?;
            }
            count += page.rows.len();
            if count >= page.total {
                break;
            }
            if page.rows.is_empty() {
                return Err(Failure::new(
                    "unavailable",
                    "dataset paging made no progress",
                ));
            }
            query.offset = count;
        }
        Ok(count)
    })?;
    Ok(Exported { path, rows, sha256 })
}

#[derive(Serialize)]
struct BundleItem {
    evidence_id: String,
    locator: Locator,
    quote: String,
    quote_verified: bool,
    source_sha256: String,
}
#[derive(Serialize)]
struct Source {
    paper_id: String,
    sha256: String,
    path: String,
}
#[derive(Serialize)]
struct Bundle {
    bundle_version: u32,
    research_id: String,
    research_version: String,
    items: Vec<BundleItem>,
    sources: Vec<Source>,
}
fn bundle(ix: &Index, ids: &[String]) -> Res<Vec<u8>> {
    let mut items = Vec::new();
    let mut sources = BTreeMap::new();
    for id in ids {
        let resolved = evidence::resolve(ix, id, &Limits::default())?;
        let hash = match (&resolved.item.locator, &resolved.target) {
            (Locator::PdfPage { paper_id, .. }, Target::PdfPage { document_path, .. }) => {
                let hash = read::hash_file(&ix.located().document_path(document_path)?)?;
                sources.insert(
                    paper_id.clone(),
                    Source {
                        paper_id: paper_id.clone(),
                        sha256: hash.clone(),
                        path: document_path.clone(),
                    },
                );
                hash
            }
            (Locator::RawField { root, file, .. }, _) => read::hash_file(&read::guard(
                &ix.located().root_path(root.as_deref())?,
                file,
            )?)?,
            _ => return Err(Failure::new("unresolved_locator", id)),
        };
        items.push(BundleItem {
            evidence_id: id.clone(),
            locator: resolved.item.locator,
            quote: resolved.item.quote,
            quote_verified: resolved.item.quote_verified,
            source_sha256: hash,
        });
    }
    let m = &ix.located().manifest;
    serde_json::to_vec(&Bundle {
        bundle_version: 1,
        research_id: m.research_id.clone(),
        research_version: m.research_version.clone(),
        items,
        sources: sources.into_values().collect(),
    })
    .map_err(io)
}
pub fn export_evidence_bundle(
    ix: &Index,
    evidence_ids: &[String],
    dest_dir: &Path,
) -> Res<Exported> {
    let dir = check_dest(ix.located(), dest_dir)?;
    let bytes = bundle(ix, evidence_ids)?;
    let mut sorted = evidence_ids.to_vec();
    sorted.sort();
    let name = format!(
        "evidence-bundle-{}.json",
        &sha256_hex(sorted.join("\n").as_bytes())[..12]
    );
    check_dest(ix.located(), &dir.join(&name))?;
    fs::create_dir_all(&dir).map_err(io)?;
    let (_, path, sha256) = atomic_stream(&dir, &name, |f| {
        f.write_all(&bytes).map_err(io)?;
        Ok(())
    })?;
    Ok(Exported {
        path,
        rows: evidence_ids.len(),
        sha256,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub snapshot_version: u32,
    pub research_id: String,
    pub research_version: String,
    pub manifest_sha256: String,
    pub data: BTreeMap<String, String>,
    pub code_commit: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub prompts: BTreeMap<String, String>,
    pub seeds: BTreeMap<String, i64>,
    pub splits: BTreeMap<String, String>,
    pub repro: repro::ReproResult,
    pub annotations_sha256: Option<String>,
    pub files: BTreeMap<String, String>,
    pub snapshot_id: String,
}
pub fn snapshot(
    ix: &Index,
    research_home: &Path,
    dest_dir: &Path,
    clock: &dyn Clock,
) -> Res<(Snapshot, PathBuf)> {
    let dir = check_dest(ix.located(), dest_dir)?;
    let home = check_dest(ix.located(), research_home)?;
    let l = ix.located();
    let m = &l.manifest;
    let annotations_sha256 = AnnotationStore::open(&home, &m.research_id)?.file_sha256()?;
    let mut data = BTreeMap::new();
    let mut splits = BTreeMap::new();
    for a in &m.artifacts {
        if let ArtifactStatus::Ready(v) = ix.status(&a.id) {
            let h = match v.sha256 {
                Some(h) => h,
                None => read::hash_file(&l.artifact_path(a)?)?,
            };
            if matches!(a.role, Role::Splits | Role::Pilot | Role::Acceptance) {
                splits.insert(a.id.clone(), h.clone());
            }
            data.insert(a.id.clone(), h);
        }
    }
    fs::create_dir_all(&dir).map_err(io)?;
    let staging = temp_path(&dir);
    fs::create_dir(&staging).map_err(io)?;
    let result = (|| {
        let mut files = BTreeMap::new();
        for (cap, layer, name) in [
            (
                Capability::Literature,
                DataLayer::Extracted,
                "dataset-extracted.csv",
            ),
            (
                Capability::RawMeasurements,
                DataLayer::Raw,
                "dataset-raw.csv",
            ),
        ] {
            if l.has(cap) {
                let x = export_csv(
                    ix,
                    &DatasetQuery {
                        layer,
                        filters: vec![],
                        sort: None,
                        offset: 0,
                        limit: 500,
                    },
                    &staging,
                    name,
                )?;
                files.insert(name.into(), x.sha256);
            }
        }
        if l.has(Capability::Evidence) {
            let a = l
                .by_role(Role::Evidence)
                .into_iter()
                .next()
                .ok_or_else(|| Failure::new("missing_artifact", "evidence missing"))?;
            let ids: Vec<String> = (0..ix.len(&a.id))
                .filter_map(|n| ix.row(&a.id, n as u32))
                .map(|r| r.get("evidence_id").to_owned())
                .collect();
            let bytes = bundle(ix, &ids)?;
            let (_, _, hash) = atomic_stream(&staging, "evidence.json", |f| {
                f.write_all(&bytes).map_err(io)?;
                Ok(())
            })?;
            files.insert("evidence.json".into(), hash);
        }
        let mut s = Snapshot {
            snapshot_version: 1,
            research_id: m.research_id.clone(),
            research_version: m.research_version.clone(),
            manifest_sha256: l.manifest_sha256.clone(),
            data,
            code_commit: m.versions.code.clone(),
            environment: m.versions.environment.clone(),
            prompts: m.versions.prompts.clone(),
            seeds: m.versions.seeds.clone(),
            splits,
            repro: repro::guard(ix),
            annotations_sha256,
            files,
            snapshot_id: String::new(),
        };
        // Value objects use sorted keys, including nested structs, for canonical JSON.
        let mut body = serde_json::to_value(&s).map_err(io)?;
        body.as_object_mut().unwrap().remove("snapshot_id");
        let hash = sha256_hex(&serde_json::to_vec(&body).map_err(io)?);
        s.snapshot_id = format!("snapshot:{}", &hash[..16]);
        let mut json = serde_json::to_value(&s).map_err(io)?;
        json.as_object_mut()
            .unwrap()
            .insert("written_at".into(), clock.now_ms().into());
        let bytes = serde_json::to_vec(&json).map_err(io)?;
        atomic_stream(&staging, "snapshot.json", |f| {
            f.write_all(&bytes).map_err(io)?;
            Ok(())
        })?;
        let target = check_dest(l, &dir.join(format!("snapshot-{}", &hash[..16])))?;
        if target.exists() {
            if !target.is_dir() {
                return Err(io("snapshot target is not a directory"));
            }
            for name in s
                .files
                .keys()
                .map(String::as_str)
                .chain(std::iter::once("snapshot.json"))
            {
                check_dest(l, &target.join(name))?;
                rename_file(&staging.join(name), &target.join(name))?;
            }
            fs::remove_dir(&staging).map_err(io)?;
        } else {
            fs::rename(&staging, &target).map_err(io)?;
        }
        annotations::register_snapshot(
            &home,
            &m.research_id,
            &SnapshotEntry {
                snapshot_id: s.snapshot_id.clone(),
                artifacts: s.data.keys().cloned().collect(),
                sha256: hash,
            },
        )?;
        Ok((s, target))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    Delivered { to: String },
    Unwired,
    Failed { kind: String, message: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandOff {
    pub consumer: Delivery,
    pub vault: Delivery,
}
fn delivery(name: String, result: Res<()>) -> Delivery {
    match result {
        Ok(()) => Delivery::Delivered { to: name },
        Err(e) => Delivery::Failed {
            kind: e.kind,
            message: e.message,
        },
    }
}
pub fn hand_off(
    s: &Snapshot,
    dir: &Path,
    consumer: Option<&dyn SnapshotConsumer>,
    vault: Option<&dyn VaultActions>,
) -> HandOff {
    let consumer = consumer.map_or(Delivery::Unwired, |c| {
        delivery(
            c.name(),
            c.consume(&SnapshotRef {
                snapshot_id: s.snapshot_id.clone(),
                research_id: s.research_id.clone(),
                research_version: s.research_version.clone(),
                path: dir.into(),
            }),
        )
    });
    let vault = vault.map_or(Delivery::Unwired, |v| {
        delivery(
            v.name(),
            v.link(&VaultLink {
                research_id: s.research_id.clone(),
                research_version: s.research_version.clone(),
                snapshot_id: Some(s.snapshot_id.clone()),
            }),
        )
    });
    HandOff { consumer, vault }
}
