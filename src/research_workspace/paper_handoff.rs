use super::{
    evidence::Locator,
    export::Snapshot,
    read,
    seams::{SnapshotConsumer, SnapshotRef},
};
use crate::{
    base::{self, Failure, Res},
    paper::{
        evidence::BundleInputs,
        http::{CurlTransport, Transport},
        service::PaperSession,
    },
    research_program::{
        self,
        dossier::{self, Confidence, Dossier, DossierItem, SourceRef},
    },
    seams::{sha256_hex, Clock, SystemClock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub struct PaperHandoff {
    axiom_home: PathBuf,
    data_home: PathBuf,
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HandoffRecord {
    pub research_id: String,
    pub research_version: String,
    pub snapshot_id: String,
    pub manifest_sha256: String,
    pub artifact_sha256: BTreeMap<String, String>,
    pub paper_store: PathBuf,
    pub evidence_hash: String,
}

pub fn store_for(axiom_home: &Path, s: &SnapshotRef) -> PathBuf {
    let name = s
        .path
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| s.snapshot_id.replace(':', "-").into());
    axiom_home.join("paper").join(&s.research_id).join(name)
}

#[derive(Deserialize)]
struct EvidenceInput {
    bundle_version: u32,
    research_id: String,
    research_version: String,
    items: Vec<EvidenceInputItem>,
}
#[derive(Deserialize)]
struct EvidenceInputItem {
    evidence_id: String,
    locator: Locator,
    quote: String,
    quote_verified: bool,
    source_sha256: String,
}

fn missing(name: &str) -> Failure {
    Failure::new("missing_input", format!("missing {name}"))
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn read_required(path: &Path, name: &str) -> Res<Vec<u8>> {
    fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            missing(name)
        } else {
            io(format!("{name}: {e}"))
        }
    })
}
fn safe_component(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['/', '\\', ':'])
        && matches!(
            (
                Path::new(s).components().next(),
                Path::new(s).components().nth(1)
            ),
            (Some(Component::Normal(_)), None)
        )
}
fn resolved_future(path: &Path) -> Res<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(io)?.join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut tail = Vec::new();
    while !ancestor.exists() {
        tail.push(
            ancestor
                .file_name()
                .ok_or_else(|| io("invalid store path"))?
                .to_owned(),
        );
        ancestor = ancestor.parent().ok_or_else(|| io("invalid store path"))?;
    }
    let mut resolved = fs::canonicalize(ancestor).map_err(io)?;
    for part in tail.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}
fn executable_available(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

impl PaperHandoff {
    pub fn new(
        axiom_home: &Path,
        data_home: &Path,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            axiom_home: axiom_home.into(),
            data_home: data_home.into(),
            transport,
            clock,
        }
    }
    pub fn production(axiom_home: &Path) -> Res<Self> {
        if std::env::var_os("AXIOM_CURL_CMD").is_some_and(|value| !value.is_empty()) {
            let command = base::resolve_command("AXIOM_CURL_CMD", "curl")?;
            let executable = Path::new(&command[0]);
            let available = if executable.components().count() > 1 {
                executable.is_file() && executable_available(executable)
            } else {
                base::on_path(&command[0]).is_some()
            };
            if !available {
                return Err(Failure::new(
                    "not_installed",
                    format!("curl command not found: {}", command[0]),
                ));
            }
        }
        Ok(Self::new(
            axiom_home,
            &base::data_home().unwrap_or_else(|_| axiom_home.to_path_buf()),
            Arc::new(CurlTransport::from_env()?),
            Arc::new(SystemClock),
        ))
    }
}
impl SnapshotConsumer for PaperHandoff {
    fn name(&self) -> String {
        "paper".into()
    }
    fn consume(&self, s: &SnapshotRef) -> Res<()> {
        if !safe_component(&s.research_id) {
            return Err(Failure::new("escape", "unsafe research_id"));
        }
        if !s.path.exists() {
            return Err(missing("snapshot.json"));
        }
        let snapshot_path = read::guard(&s.path, "snapshot.json")?;
        let bytes = read_required(&snapshot_path, "snapshot.json")?;
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|e| Failure::new("json", format!("snapshot.json: {e}")))?;
        let mut canonical =
            serde_json::to_value(&snapshot).map_err(|e| Failure::new("json", e.to_string()))?;
        canonical.as_object_mut().unwrap().remove("snapshot_id");
        let hash = sha256_hex(
            &serde_json::to_vec(&canonical).map_err(|e| Failure::new("json", e.to_string()))?,
        );
        if snapshot.snapshot_id != format!("snapshot:{}", &hash[..16])
            || snapshot.snapshot_id != s.snapshot_id
        {
            return Err(Failure::new("corrupt_artifact", "snapshot_id mismatch"));
        }
        if snapshot.research_id != s.research_id || snapshot.research_version != s.research_version
        {
            return Err(Failure::new(
                "corrupt_artifact",
                "snapshot research identity mismatch",
            ));
        }
        if !snapshot.files.contains_key("evidence.json") {
            return Err(missing("evidence.json"));
        }
        let mut files = BTreeMap::new();
        for (name, expected) in &snapshot.files {
            let path = read::guard(&s.path, name)?;
            let bytes = read_required(&path, name)?;
            if !sha256_hex(&bytes).eq_ignore_ascii_case(expected) {
                return Err(Failure::new(
                    "corrupt_artifact",
                    format!("sha256 mismatch: {name}"),
                ));
            }
            files.insert(name.clone(), bytes);
        }
        let evidence: EvidenceInput = serde_json::from_slice(&files["evidence.json"])
            .map_err(|e| Failure::new("json", format!("evidence.json: {e}")))?;
        if evidence.bundle_version != 1
            || evidence.research_id != snapshot.research_id
            || evidence.research_version != snapshot.research_version
        {
            return Err(Failure::new(
                "corrupt_artifact",
                "evidence.json identity or version mismatch",
            ));
        }
        let mut items = Vec::new();
        for item in evidence.items {
            let source = match &item.locator {
                Locator::PdfPage { paper_id, .. } => paper_id.clone(),
                Locator::RawField { file, .. } => file.clone(),
            };
            let confidence = if item.quote_verified {
                Confidence::Medium
            } else {
                Confidence::Low
            };
            let locator = serde_json::to_string(&item.locator)
                .map_err(|e| Failure::new("json", e.to_string()))?;
            items.push(DossierItem {
                id: item.evidence_id,
                claim: item.quote,
                sources: vec![SourceRef {
                    source,
                    author: snapshot.research_id.clone(),
                    confidence,
                    evidence: Some(format!("{locator}; sha256:{}", item.source_sha256)),
                }],
                confidence,
                independent_support: 1,
                contradictions: vec![],
            });
        }
        let dossier = Dossier {
            question: snapshot.research_id.clone(),
            items,
            rejected: vec![],
        };
        let store = store_for(&self.axiom_home, s);
        let source = fs::canonicalize(&s.path).map_err(io)?;
        let target = resolved_future(&store)?;
        if target.starts_with(&source) || source.starts_with(&target) {
            return Err(Failure::new(
                "dest_in_source",
                "paper store overlaps snapshot",
            ));
        }
        fs::create_dir_all(&store).map_err(io)?;
        read::guard(&store, "research/program/02-dossier.json")?;
        read::guard(&store, "research/paper/evidence.json")?;
        read::guard(&store, "research/paper/state.json")?;
        read::guard(&store, "handoff.json")?;
        fs::create_dir_all(research_program::program_dir(&store)).map_err(io)?;
        dossier::write(
            &research_program::program_dir(&store).join("02-dossier.json"),
            &dossier,
        )?;
        let mut inputs = BundleInputs {
            store: store.clone(),
            project_root: store.clone(),
            results: vec![],
            datasets: vec![],
            models: vec![],
        };
        for (name, bytes) in files {
            let file_name = Path::new(&name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let paths = if file_name.starts_with("dataset-") && file_name.ends_with(".csv") {
                &mut inputs.datasets
            } else if file_name == "result.json"
                || file_name == "results.json"
                || (file_name.starts_with("result-") && file_name.ends_with(".json"))
                || (file_name.starts_with("results-") && file_name.ends_with(".json"))
            {
                &mut inputs.results
            } else if file_name.starts_with("model-") || file_name.starts_with("model.") {
                &mut inputs.models
            } else {
                continue;
            };
            let relative = Path::new("inputs").join(&name);
            let dest = read::guard(&store, &relative.to_string_lossy().replace('\\', "/"))?;
            fs::create_dir_all(dest.parent().unwrap()).map_err(io)?;
            fs::write(dest, bytes).map_err(io)?;
            paths.push(relative);
        }
        let session = PaperSession::open(
            &store,
            &self.data_home,
            &store,
            self.transport.clone(),
            self.clock.clone(),
        )?;
        let bundle = session.build_evidence(&inputs)?;
        let record = HandoffRecord {
            research_id: snapshot.research_id,
            research_version: snapshot.research_version,
            snapshot_id: snapshot.snapshot_id,
            manifest_sha256: snapshot.manifest_sha256,
            artifact_sha256: snapshot.data,
            paper_store: store.clone(),
            evidence_hash: bundle.hash,
        };
        let bytes =
            serde_json::to_vec_pretty(&record).map_err(|e| Failure::new("json", e.to_string()))?;
        base::atomic_write(&store.join("handoff.json"), &bytes).map_err(io)?;
        Ok(())
    }
}
