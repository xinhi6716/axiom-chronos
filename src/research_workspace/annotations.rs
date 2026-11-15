use super::{
    index::Index,
    manifest::{split_record_id, Role},
    read,
};
use crate::{
    base::{Failure, Res},
    seams::{sha256_hex, Clock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Confirm,
    Dispute { reason: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotCheck {
    pub id: String,
    pub target: String,
    pub node: Option<String>,
    pub verdict: Verdict,
    pub who: String,
    pub at_ms: i64,
    pub research_version: String,
    pub target_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reverify {
    Pending,
    Passed,
    Failed { reason: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRef {
    pub client_id: String,
    pub session: String,
    pub stage: String,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionDraft {
    pub target: String,
    pub field: String,
    pub after: String,
    pub who: String,
    pub why: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    pub id: String,
    pub target: String,
    pub field: String,
    pub before: String,
    pub after: String,
    pub who: String,
    pub why: String,
    pub at_ms: i64,
    pub research_version: String,
    pub target_sha256: String,
    pub reverify: Reverify,
    pub invalidates: Vec<String>,
    pub receipt: Option<ReceiptRef>,
    pub draft: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub snapshot_id: String,
    pub artifacts: Vec<String>,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    SpotCheck {
        #[serde(flatten)]
        value: SpotCheck,
    },
    Correction {
        #[serde(flatten)]
        value: Correction,
    },
    State {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reverify: Option<Reverify>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receipt: Option<ReceiptRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        draft: Option<String>,
    },
}
fn path(home: &Path, id: &str, category: &str, name: &str) -> Res<PathBuf> {
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\', ':', '\0']) {
        return Err(Failure::new("invalid_argument", "invalid research id"));
    }
    // Guard existing components, including symlinks, before any write.
    if home.exists() {
        read::guard(home, &format!("{category}/{id}/{name}"))
    } else {
        Ok(home.join(category).join(id).join(name))
    }
}
fn lines<T: for<'de> Deserialize<'de>>(p: &Path) -> Vec<T> {
    let Ok(f) = fs::File::open(p) else {
        return Vec::new();
    };
    BufReader::new(f)
        .split(b'\n')
        .filter_map(|x| x.ok())
        .filter_map(|x| serde_json::from_slice(&x).ok())
        .collect()
}
fn append<T: Serialize>(p: &Path, value: &T) -> Res<()> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| Failure::new("io", e.to_string()))?;
    bytes.push(b'\n');
    fs::create_dir_all(p.parent().unwrap()).map_err(|e| Failure::new("io", e.to_string()))?;
    // Keep a previously truncated final record separate from the next full line.
    if let Ok(mut f) = fs::File::open(p) {
        use std::io::{Read, Seek, SeekFrom};
        if f.seek(SeekFrom::End(-1)).is_ok() {
            let mut last = [0];
            if f.read_exact(&mut last).is_ok() && last[0] != b'\n' {
                bytes.insert(0, b'\n');
            }
        }
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .map_err(|e| Failure::new("io", e.to_string()))?;
    match f.write(&bytes) {
        Ok(n) if n == bytes.len() => Ok(()),
        Ok(_) => Err(Failure::new("io", "incomplete append")),
        Err(e) => Err(Failure::new("io", e.to_string())),
    }
}
fn unique_id(target: &str, who: &str, at: i64) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    sha256_hex(format!("{target}\0{who}\0{at}\0{n}\0{t}\0{}", std::process::id()).as_bytes())
}
fn target<'a>(
    ix: &'a Index,
    id: &str,
) -> Res<(&'a super::manifest::Artifact, super::index::RowRef<'a>)> {
    let (artifact, key) =
        split_record_id(id).ok_or_else(|| Failure::new("invalid_argument", "unknown target"))?;
    let a = ix
        .located()
        .artifact(artifact)
        .ok_or_else(|| Failure::new("invalid_argument", "unknown target"))?;
    let row = ix
        .find(artifact, key)
        .and_then(|i| ix.row(artifact, i))
        .ok_or_else(|| Failure::new("invalid_argument", "unknown target"))?;
    Ok((a, row))
}
fn source_hash(ix: &Index, a: &super::manifest::Artifact) -> Res<String> {
    read::hash_file(&ix.located().artifact_path(a)?)
}

pub struct AnnotationStore {
    home: PathBuf,
    research_id: String,
    file: PathBuf,
}
impl AnnotationStore {
    pub fn open(research_home: &Path, research_id: &str) -> Res<Self> {
        Ok(Self {
            home: research_home.to_owned(),
            research_id: research_id.into(),
            file: path(
                research_home,
                research_id,
                "annotations",
                "annotations.jsonl",
            )?,
        })
    }
    fn read(&self) -> (Vec<SpotCheck>, Vec<Correction>) {
        let mut spots = Vec::new();
        let mut corrections: Vec<Correction> = Vec::new();
        let mut ids = BTreeMap::new();
        for line in lines::<Line>(&self.file) {
            match line {
                Line::SpotCheck { value } => spots.push(value),
                Line::Correction { value } => {
                    ids.insert(value.id.clone(), corrections.len());
                    corrections.push(value);
                }
                Line::State {
                    id,
                    reverify,
                    receipt,
                    draft,
                } => {
                    if let Some(&i) = ids.get(&id) {
                        let c: &mut Correction = &mut corrections[i];
                        if let Some(v) = reverify {
                            c.reverify = v;
                        }
                        if let Some(v) = receipt {
                            c.receipt = Some(v);
                        }
                        if let Some(v) = draft {
                            c.draft = Some(v);
                        }
                    }
                }
            }
        }
        (spots, corrections)
    }
    fn write(&self, line: &Line) -> Res<()> {
        let p = path(
            &self.home,
            &self.research_id,
            "annotations",
            "annotations.jsonl",
        )?;
        append(&p, line)
    }
    pub fn spot_check(
        &mut self,
        ix: &Index,
        id: &str,
        node: Option<&str>,
        v: Verdict,
        who: &str,
        clock: &dyn Clock,
    ) -> Res<SpotCheck> {
        let (a, _) = target(ix, id)?;
        let at_ms = clock.now_ms();
        let value = SpotCheck {
            id: unique_id(id, who, at_ms),
            target: id.into(),
            node: node.map(str::to_owned),
            verdict: v,
            who: who.into(),
            at_ms,
            research_version: ix.located().manifest.research_version.clone(),
            target_sha256: source_hash(ix, a)?,
        };
        self.write(&Line::SpotCheck {
            value: value.clone(),
        })?;
        Ok(value)
    }
    pub fn correct(
        &mut self,
        ix: &Index,
        d: CorrectionDraft,
        clock: &dyn Clock,
    ) -> Res<Correction> {
        if d.who.trim().is_empty() || d.why.trim().is_empty() {
            return Err(Failure::new("invalid_argument", "who and why are required"));
        }
        let (a, row) = target(ix, &d.target)?;
        if if a.columns.is_empty() {
            ix.column_values(&a.id, &d.field, 1).is_empty()
        } else {
            !a.columns.contains_key(&d.field)
        } {
            return Err(Failure::new("invalid_argument", "unknown field"));
        }
        let at_ms = clock.now_ms();
        let value = Correction {
            id: unique_id(&d.target, &d.who, at_ms),
            before: row.get(&d.field).into(),
            invalidates: downstream(ix, &d.target, &snapshots(&self.home, &self.research_id)),
            target_sha256: source_hash(ix, a)?,
            research_version: ix.located().manifest.research_version.clone(),
            target: d.target,
            field: d.field,
            after: d.after,
            who: d.who,
            why: d.why,
            at_ms,
            reverify: Reverify::Pending,
            receipt: None,
            draft: None,
        };
        self.write(&Line::Correction {
            value: value.clone(),
        })?;
        Ok(value)
    }
    fn state(
        &self,
        id: &str,
        reverify: Option<Reverify>,
        receipt: Option<ReceiptRef>,
        draft: Option<String>,
    ) -> Res<()> {
        if !self.read().1.iter().any(|c| c.id == id) {
            return Err(Failure::new("not_found", "unknown correction"));
        }
        self.write(&Line::State {
            id: id.into(),
            reverify,
            receipt,
            draft,
        })
    }
    pub fn set_reverify(&mut self, id: &str, v: Reverify) -> Res<()> {
        self.state(id, Some(v), None, None)
    }
    pub fn set_receipt(&mut self, id: &str, v: ReceiptRef) -> Res<()> {
        self.state(id, None, Some(v), None)
    }
    pub fn keep_draft(&mut self, id: &str, text: &str) -> Res<()> {
        self.state(id, None, None, Some(text.into()))
    }
    pub fn spot_checks(&self, target: Option<&str>) -> Vec<SpotCheck> {
        self.read()
            .0
            .into_iter()
            .filter(|s| target.map_or(true, |t| s.target == t))
            .collect()
    }
    pub fn corrections(&self, target: Option<&str>) -> Vec<Correction> {
        self.read()
            .1
            .into_iter()
            .filter(|c| target.map_or(true, |t| c.target == t))
            .collect()
    }
    pub fn invalidated(&self) -> BTreeSet<String> {
        self.read()
            .1
            .into_iter()
            .flat_map(|c| c.invalidates)
            .collect()
    }
    pub fn file_sha256(&self) -> Res<Option<String>> {
        match read::hash_file(&self.file) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind == "missing_artifact" => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn tokens<'a>(s: &'a str, sep: Option<&str>) -> Vec<&'a str> {
    match sep.filter(|s| !s.is_empty()) {
        Some(sep) => s
            .split(sep)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect(),
        None => {
            let s = s.trim();
            if s.is_empty() {
                vec![]
            } else {
                vec![s]
            }
        }
    }
}
pub fn downstream(ix: &Index, target_id: &str, snapshots: &[SnapshotEntry]) -> Vec<String> {
    let Ok((a, _)) = target(ix, target_id) else {
        return Vec::new();
    };
    let mut visited = BTreeSet::from([target_id.to_owned()]);
    let mut artifacts = BTreeSet::from([a.id.clone()]);
    let mut queue = VecDeque::from([target_id.to_owned()]);
    while let Some(id) = queue.pop_front() {
        let Some((from, key)) = split_record_id(&id) else {
            continue;
        };
        let Some(row) = ix.find(from, key).and_then(|i| ix.row(from, i)) else {
            continue;
        };
        for link in ix
            .located()
            .manifest
            .links
            .iter()
            .filter(|l| l.from == from)
        {
            let from_tokens = tokens(row.get(&link.key), link.separator.as_deref());
            if from_tokens.is_empty() {
                continue;
            }
            for i in 0..ix.len(&link.to) {
                let Some(other) = ix.row(&link.to, i as u32) else {
                    continue;
                };
                if tokens(other.get(&link.key), link.separator.as_deref())
                    .iter()
                    .any(|t| from_tokens.contains(t))
                {
                    let next = other.record_id();
                    artifacts.insert(link.to.clone());
                    if visited.insert(next.clone()) {
                        queue.push_back(next);
                    }
                }
            }
        }
    }
    visited.remove(target_id);
    for s in snapshots {
        if s.artifacts.iter().any(|a| artifacts.contains(a)) && s.snapshot_id != target_id {
            visited.insert(s.snapshot_id.clone());
        }
    }
    visited.into_iter().collect()
}
pub fn reverify(ix: &Index, c: &Correction) -> Reverify {
    let fail = |s: &str| Reverify::Failed { reason: s.into() };
    let Ok((a, row)) = target(ix, &c.target) else {
        return fail("The target is gone.");
    };
    match source_hash(ix, a) {
        Ok(hash) if hash == c.target_sha256 => {}
        _ => return fail("The target source changed."),
    }
    if a.role == Role::Evidence
        && [
            "physical_page",
            "printed_page",
            "locator_id",
            "raw_file",
            "raw_root",
            "raw_row",
            "raw_key",
            "raw_column",
            "bbox",
        ]
        .contains(&c.field.as_str())
    {
        let field = |name: &str| {
            if c.field == name {
                c.after.as_str()
            } else {
                row.get(name)
            }
        };
        if c.field == "physical_page"
            && field("physical_page")
                .parse::<u64>()
                .ok()
                .filter(|n| *n >= 1)
                .is_none()
        {
            return fail("The physical page is invalid.");
        }
        let raw_file = field("raw_file");
        let raw_root = field("raw_root");
        if !raw_file.is_empty() || !raw_root.is_empty() || row.get("locator_type") == "raw_field" {
            if raw_file.is_empty()
                || raw_root.is_empty()
                || ix
                    .located()
                    .root_path(Some(raw_root))
                    .and_then(|p| read::guard(&p, raw_file))
                    .map_or(true, |p| !p.is_file())
            {
                return fail("The raw source cannot be resolved.");
            }
        }
    }
    Reverify::Passed
}
pub fn reverify_request(c: &Correction, session: &str) -> (String, String) {
    (
        format!("research-reverify-{}", c.id),
        format!(
            "Reverify target {} in session {}. Field: {}. Before: {}. After: {}. Who: {}. Why: {}.",
            c.target, session, c.field, c.before, c.after, c.who, c.why
        ),
    )
}
pub fn register_snapshot(research_home: &Path, research_id: &str, e: &SnapshotEntry) -> Res<()> {
    append(
        &path(research_home, research_id, "snapshots", "registry.jsonl")?,
        e,
    )
}
pub fn snapshots(research_home: &Path, research_id: &str) -> Vec<SnapshotEntry> {
    let Ok(p) = path(research_home, research_id, "snapshots", "registry.jsonl") else {
        return Vec::new();
    };
    let all: Vec<SnapshotEntry> = lines(&p);
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for e in all.into_iter().rev() {
        if seen.insert(e.snapshot_id.clone()) {
            result.push(e);
        }
    }
    result.reverse();
    result
}
