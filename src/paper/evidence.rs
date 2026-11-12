use super::{csv, model::*};
use crate::{
    base::{atomic_write, Failure, Res},
    research_program::{self, dossier},
    seams::sha256_hex,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub struct BundleInputs {
    pub store: PathBuf,
    pub project_root: PathBuf,
    pub results: Vec<PathBuf>,
    pub datasets: Vec<PathBuf>,
    pub models: Vec<PathBuf>,
}

fn input(
    root: &Path,
    path: &Path,
    kind: ArtifactKind,
    producer: &str,
) -> Res<(ArtifactRef, Vec<u8>)> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    };
    let canonical = fs::canonicalize(&path).map_err(|e| {
        Failure::new(
            if e.kind() == std::io::ErrorKind::NotFound {
                "missing_input"
            } else {
                "io"
            },
            format!("{}: {e}", path.display()),
        )
    })?;
    let relative = canonical.strip_prefix(root).map_err(|_| {
        Failure::new(
            "path_escape",
            format!("{} is outside project root", path.display()),
        )
    })?;
    let bytes = fs::read(&canonical).map_err(|e| Failure::new("io", e.to_string()))?;
    Ok((
        ArtifactRef {
            path: relative.to_string_lossy().replace('\\', "/"),
            sha256: sha256_hex(&bytes),
            kind,
            producer: producer.into(),
        },
        bytes,
    ))
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}
fn reference(source: &str) -> Option<ReferenceSeed> {
    let source = source.trim();
    let doi_re = regex::Regex::new(r"(?i)^10\.\d{4,9}/\S+$").unwrap();
    let id_re = regex::Regex::new(r"(?i)^W[0-9]+$").unwrap();
    let doi = source
        .strip_prefix("https://doi.org/")
        .or_else(|| source.strip_prefix("http://doi.org/"))
        .or_else(|| source.strip_prefix("doi:"))
        .unwrap_or(source);
    if doi_re.is_match(doi) {
        Some(ReferenceSeed {
            key: slug(doi),
            doi: Some(doi.into()),
            openalex: None,
            url: None,
            evidence: Vec::new(),
        })
    } else {
        let id = source
            .strip_prefix("https://openalex.org/")
            .or_else(|| source.strip_prefix("http://openalex.org/"))
            .unwrap_or(source);
        if id_re.is_match(id) {
            let id = id.to_ascii_uppercase();
            Some(ReferenceSeed {
                key: slug(&id),
                doi: None,
                openalex: Some(id),
                url: None,
                evidence: Vec::new(),
            })
        } else if source.starts_with("https://") || source.starts_with("http://") {
            Some(ReferenceSeed {
                key: slug(source),
                doi: None,
                openalex: None,
                url: Some(source.into()),
                evidence: Vec::new(),
            })
        } else {
            None
        }
    }
}

pub fn build(inputs: &BundleInputs) -> Res<EvidenceBundle> {
    let root = fs::canonicalize(&inputs.project_root)
        .map_err(|e| Failure::new("missing_input", e.to_string()))?;
    let program_dir = research_program::program_dir(&inputs.store);
    let dossier_path = program_dir.join("02-dossier.json");
    // Resolve store-relative paths against the working directory, not the project root.
    let dossier_path = if dossier_path.is_absolute() {
        dossier_path
    } else {
        std::env::current_dir()
            .map_err(|e| Failure::new("io", e.to_string()))?
            .join(dossier_path)
    };
    let (artifact, _) = input(
        &root,
        &dossier_path,
        ArtifactKind::Dossier,
        "research_program",
    )?;
    let dossier = dossier::read(&dossier_path)?;
    let mut artifacts = vec![artifact];
    let adjudication = dossier_path.with_file_name("07-adjudication.json");
    match fs::metadata(&adjudication) {
        Ok(_) => {
            let (artifact, _) = input(
                &root,
                &adjudication,
                ArtifactKind::Adjudication,
                "research_program",
            )?;
            dossier::read(&adjudication)?;
            artifacts.push(artifact);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Failure::new("io", e.to_string())),
    }
    let mut claims = Vec::new();
    let mut evidence = Vec::new();
    let mut references: Vec<ReferenceSeed> = Vec::new();
    for item in &dossier.items {
        let mut ids = Vec::new();
        for source in &item.sources {
            let id = format!("e{}", evidence.len() + 1);
            if let Some(mut reference) = reference(&source.source) {
                if let Some(existing) = references.iter_mut().find(|r| r.key == reference.key) {
                    existing.evidence.push(id.clone());
                } else {
                    reference.evidence.push(id.clone());
                    references.push(reference);
                }
            }
            ids.push(id.clone());
            evidence.push(EvidenceItem {
                id,
                source: source.source.clone(),
                artifact: None,
                locator: source.evidence.clone(),
            });
        }
        claims.push(Claim {
            id: item.id.clone(),
            text: item.claim.clone(),
            evidence: ids,
            confidence: item.confidence,
        });
    }
    let mut quantities = Vec::new();
    for path in &inputs.results {
        let (mut artifact, bytes) = input(&root, path, ArtifactKind::Result, "")?;
        let result: ResultsFile =
            serde_json::from_slice(&bytes).map_err(|e| Failure::new("json", e.to_string()))?;
        artifact.producer = result.producer;
        artifacts.push(artifact);
        quantities.extend(result.quantities);
    }
    for (paths, kind) in [
        (&inputs.datasets, ArtifactKind::Dataset),
        (&inputs.models, ArtifactKind::Model),
    ] {
        for path in paths {
            artifacts.push(input(&root, path, kind.clone(), "input")?.0);
        }
    }
    let claim_ids: BTreeSet<_> = claims.iter().map(|c| c.id.as_str()).collect();
    let evidence_ids: BTreeSet<_> = evidence.iter().map(|e| e.id.as_str()).collect();
    if claims
        .iter()
        .flat_map(|claim| &claim.evidence)
        .any(|id| !evidence_ids.contains(id.as_str()))
    {
        return Err(Failure::new("dangling", "unknown evidence in claim"));
    }
    for q in &quantities {
        if q.claim_ids
            .iter()
            .any(|id| !claim_ids.contains(id.as_str()))
        {
            return Err(Failure::new(
                "dangling",
                format!("unknown claim in quantity {}", q.id),
            ));
        }
        if !artifacts.iter().any(|a| a.path == q.locator.artifact) {
            return Err(Failure::new(
                "dangling",
                format!("unknown artifact {}", q.locator.artifact),
            ));
        }
    }
    let mut bundle = EvidenceBundle {
        version: 1,
        program: inputs
            .store
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        question: dossier.question,
        artifacts,
        claims,
        evidence,
        quantities,
        references,
        hash: String::new(),
    };
    bundle.hash = bundle.compute_hash();
    Ok(bundle)
}
pub fn save(store: &Path, bundle: &EvidenceBundle) -> Res<()> {
    let dir = paper_dir(store);
    fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes =
        serde_json::to_vec_pretty(bundle).map_err(|e| Failure::new("json", e.to_string()))?;
    atomic_write(&dir.join("evidence.json"), &bytes).map_err(|e| Failure::new("io", e.to_string()))
}
pub fn load(store: &Path) -> Res<Option<EvidenceBundle>> {
    match fs::read(paper_dir(store).join("evidence.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Failure::new("json", e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Failure::new("io", e.to_string())),
    }
}
fn checked_bytes(root: &Path, artifact: &ArtifactRef) -> Res<Vec<u8>> {
    let root = fs::canonicalize(root).map_err(|e| Failure::new("io", e.to_string()))?;
    let path = root.join(&artifact.path);
    let canonical = fs::canonicalize(&path)
        .map_err(|e| Failure::new("artifact_modified", format!("{}: {e}", path.display())))?;
    if !canonical.starts_with(&root) {
        return Err(Failure::new(
            "path_escape",
            format!("{} is outside project root", path.display()),
        ));
    }
    let bytes =
        fs::read(&canonical).map_err(|e| Failure::new("artifact_modified", e.to_string()))?;
    if sha256_hex(&bytes) != artifact.sha256 {
        return Err(Failure::new("artifact_modified", artifact.path.clone()));
    }
    Ok(bytes)
}
pub fn verify(bundle: &EvidenceBundle, project_root: &Path) -> Res<()> {
    if bundle.hash != bundle.compute_hash() {
        return Err(Failure::new("bundle_mismatch", "bundle hash differs"));
    }
    for artifact in &bundle.artifacts {
        checked_bytes(project_root, artifact)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataTable {
    pub columns: Vec<String>,
    pub units: Vec<Option<String>>,
    pub rows: Vec<Vec<String>>,
}

pub fn load_table(bundle: &EvidenceBundle, project_root: &Path, artifact: &str) -> Res<DataTable> {
    let artifact = bundle
        .artifacts
        .iter()
        .find(|a| a.path == artifact)
        .ok_or_else(|| Failure::new("dangling", format!("unknown artifact {artifact}")))?;
    let bytes = checked_bytes(project_root, artifact)?;
    let text = std::str::from_utf8(&bytes).map_err(|e| Failure::new("csv", e.to_string()))?;
    let delimiter = if artifact.path.to_ascii_lowercase().ends_with(".tsv") {
        b'\t'
    } else {
        b','
    };
    let table = csv::parse(text, delimiter)?;
    let (columns, units) = table.headers.iter().map(|h| csv::split_unit(h)).unzip();
    Ok(DataTable {
        columns,
        units,
        rows: table.rows,
    })
}
