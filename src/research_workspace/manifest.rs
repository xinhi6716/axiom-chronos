use super::{read, rram};
use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub const MANIFEST_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "research-manifest.json";
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub manifest_version: u32,
    pub research_id: String,
    pub research_version: String,
    pub title: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub roots: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    pub settings: serde_json::Value,
    pub versions: Versions,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub stages: Vec<StageDecl>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub groups: Option<GroupDecl>,
    #[serde(default)]
    pub stall_after_s: Option<u64>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    PdfPages,
    RawMeasurements,
    Literature,
    Screening,
    Evidence,
    Dataset,
    Pilot,
    Acceptance,
    Splits,
    Progress,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Versions {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub prompts: BTreeMap<String, String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub seeds: BTreeMap<String, i64>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Inventory,
    Screening,
    Aliases,
    Evidence,
    Measurements,
    RawMeasurements,
    Progress,
    Driver,
    Completed,
    Pilot,
    Acceptance,
    Splits,
    Validation,
    Gate,
    Protocol,
    RawSummary,
    Other,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Csv,
    Jsonl,
    Json,
    Text,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Artifact {
    pub id: String,
    pub role: Role,
    #[serde(default)]
    pub root: Option<String>,
    pub path: String,
    pub format: Format,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub columns: BTreeMap<String, String>,
    #[serde(default)]
    pub documents_root: Option<String>,
    #[serde(default)]
    pub produced: Option<Produced>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Produced {
    #[serde(default)]
    pub inputs: Vec<InputPin>,
    #[serde(default)]
    pub code_version: Option<String>,
    #[serde(default)]
    pub prompt_version: Option<String>,
    #[serde(default)]
    pub at: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InputPin {
    pub artifact: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub sha256_from: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StageDecl {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub progress: Option<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub gate: Option<GateDecl>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GateDecl {
    pub artifact: String,
    pub pointer: String,
    pub op: GateOp,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub setting: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GateOp {
    Eq,
    Ge,
    Le,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Link {
    pub from: String,
    pub to: String,
    pub key: String,
    #[serde(default)]
    pub separator: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GroupDecl {
    pub pilot: String,
    pub acceptance: String,
    pub by: GroupBy,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    Column(String),
    StudyGroups,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestSource {
    File { sha256: String },
    Adapter { name: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Located {
    pub root: PathBuf,
    pub store: Option<PathBuf>,
    pub manifest: Manifest,
    pub source: ManifestSource,
    pub manifest_sha256: String,
}
impl Located {
    pub fn artifact(&self, id: &str) -> Option<&Artifact> {
        self.manifest.artifacts.iter().find(|a| a.id == id)
    }
    pub fn by_role(&self, r: Role) -> Vec<&Artifact> {
        self.manifest
            .artifacts
            .iter()
            .filter(|a| a.role == r)
            .collect()
    }
    pub fn has(&self, c: Capability) -> bool {
        self.manifest.capabilities.contains(&c)
    }
    pub fn require(&self, c: Capability) -> Res<()> {
        if self.has(c.clone()) {
            Ok(())
        } else {
            Err(Failure::new(
                "not_capable",
                format!(
                    "capability {} ({c:?}) unavailable",
                    serde_json::to_value(&c).unwrap().as_str().unwrap()
                ),
            ))
        }
    }
    pub fn root_path(&self, name: Option<&str>) -> Res<PathBuf> {
        let Some(name) = name else {
            return Ok(self.root.clone());
        };
        if name == "project" {
            return Ok(self.root.clone());
        }
        let p = self
            .manifest
            .roots
            .get(name)
            .ok_or_else(|| Failure::new("invalid_manifest", format!("unknown root {name}")))?;
        let p = Path::new(p);
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        };
        std::fs::canonicalize(&p).map_err(|e| Failure::new("io", format!("{}: {e}", p.display())))
    }
    pub fn artifact_path(&self, a: &Artifact) -> Res<PathBuf> {
        read::guard(&self.root_path(a.root.as_deref())?, &a.path)
    }
    pub fn document_path(&self, inventory_path: &str) -> Res<PathBuf> {
        let a = self
            .by_role(Role::Inventory)
            .into_iter()
            .next()
            .ok_or_else(|| Failure::new("missing_artifact", "inventory missing"))?;
        read::guard(
            &self.root_path(a.documents_root.as_deref())?,
            inventory_path,
        )
    }
    pub fn roots(&self) -> Vec<PathBuf> {
        let mut out = vec![self.root.clone()];
        for n in self.manifest.roots.keys() {
            if let Ok(p) = self.root_path(Some(n)) {
                if !out.contains(&p) {
                    out.push(p)
                }
            }
        }
        if let Some(p) = &self.store {
            if !out.contains(p) {
                out.push(p.clone())
            }
        }
        out
    }
}
#[derive(Clone, Debug, Default)]
pub struct LocateOptions {
    pub store: Option<PathBuf>,
    pub orch_home: Option<PathBuf>,
}
pub fn locate(root: &Path, o: &LocateOptions) -> Res<Located> {
    let root = std::fs::canonicalize(root).map_err(|e| Failure::new("io", e.to_string()))?;
    let p = root.join(MANIFEST_FILE);
    let (manifest, source, hash) = if p.is_file() {
        let data = std::fs::read(&p).map_err(|e| Failure::new("io", e.to_string()))?;
        let value: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|e| Failure::new("invalid_manifest", e.to_string()))?;
        if let Some(v) = value.get("manifest_version").filter(|v| v.is_number()) {
            if v != &serde_json::json!(MANIFEST_VERSION) {
                return Err(Failure::new(
                    "unknown_version",
                    format!("unknown manifest version {v}"),
                ));
            }
        }
        let m: Manifest = serde_json::from_value(value)
            .map_err(|e| Failure::new("invalid_manifest", e.to_string()))?;
        let hash = crate::seams::sha256_hex(&data);
        (
            m,
            ManifestSource::File {
                sha256: hash.clone(),
            },
            hash,
        )
    } else if rram::detect(&root) {
        let m = rram::manifest(&root, o)?;
        let hash = crate::seams::sha256_hex(
            &serde_json::to_vec(&m).map_err(|e| Failure::new("invalid_manifest", e.to_string()))?,
        );
        (
            m,
            ManifestSource::Adapter {
                name: "rram".into(),
            },
            hash,
        )
    } else {
        return Err(Failure::new(
            "no_manifest",
            format!("no manifest in {}", root.display()),
        ));
    };
    crate::base::require_slug("research_id", &manifest.research_id)
        .map_err(|e| Failure::new("invalid_manifest", e.message))?;
    if !manifest.settings.is_object() {
        return Err(Failure::new(
            "invalid_manifest",
            "settings must be an object",
        ));
    }
    let store = o.store.clone().or_else(|| {
        o.orch_home
            .as_ref()
            .map(|p| p.join(&manifest.research_id))
            .filter(|p| p.is_dir())
    });
    Ok(Located {
        root,
        store,
        manifest,
        source,
        manifest_sha256: hash,
    })
}
pub fn record_id(artifact: &str, key: &str) -> String {
    format!("{artifact}:{key}")
}
pub fn split_record_id(id: &str) -> Option<(&str, &str)> {
    id.split_once(':')
        .filter(|(a, k)| !a.is_empty() && !k.is_empty())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locate_version_and_slug() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST_FILE), r#"{"manifest_version":2}"#).unwrap();
        assert_eq!(
            locate(dir.path(), &LocateOptions::default())
                .unwrap_err()
                .kind,
            "unknown_version"
        );
        assert_eq!(
            split_record_id(&record_id("inventory", "a:b")),
            Some(("inventory", "a:b"))
        );
    }
}
#[cfg(test)]
mod extra_tests {
    use super::*;
    #[test]
    fn negative_manifest_version_is_unknown() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(MANIFEST_FILE), r#"{"manifest_version":-1}"#).unwrap();
        assert_eq!(
            locate(d.path(), &Default::default()).unwrap_err().kind,
            "unknown_version"
        );
    }
    #[test]
    fn shape_slug_and_capabilities() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(MANIFEST_FILE);
        let mut m = serde_json::json!({"manifest_version":1,"research_id":"bad id","research_version":"v","title":"T","settings":{},"versions":{}});
        std::fs::write(&p, m.to_string()).unwrap();
        assert_eq!(
            locate(d.path(), &Default::default()).unwrap_err().kind,
            "invalid_manifest"
        );
        m["research_id"] = "valid".into();
        std::fs::write(&p, m.to_string()).unwrap();
        let l = locate(d.path(), &Default::default()).unwrap();
        let e = l.require(Capability::PdfPages).unwrap_err();
        assert_eq!(e.kind, "not_capable");
        assert!(e.message.contains("pdf_pages"), "{}", e.message);
        assert_eq!(
            l.manifest_sha256,
            crate::seams::sha256_hex(&std::fs::read(p).unwrap())
        );
    }
    #[test]
    fn locate_adapter_stores_and_paths() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            locate(d.path(), &Default::default()).unwrap_err().kind,
            "no_manifest"
        );
        std::fs::create_dir_all(d.path().join("data")).unwrap();
        std::fs::create_dir_all(d.path().join("research")).unwrap();
        std::fs::create_dir_all(d.path().join("pdfs")).unwrap();
        std::fs::write(d.path().join("data/literature_inventory.csv"), "").unwrap();
        std::fs::write(
            d.path().join("research/protocol.yaml"),
            "meta: {project: example}\npaths: {source_root: pdfs}\n",
        )
        .unwrap();
        let home = d.path().join("orch");
        let store = home.join("example");
        std::fs::create_dir_all(&store).unwrap();
        let explicit = d.path().join("explicit");
        let opts = LocateOptions {
            store: Some(explicit.clone()),
            orch_home: Some(home.clone()),
        };
        let l = locate(d.path(), &opts).unwrap();
        assert_eq!(
            l.source,
            ManifestSource::Adapter {
                name: "rram".into()
            }
        );
        assert_eq!(l.store.as_ref(), Some(&explicit));
        let l = locate(
            d.path(),
            &LocateOptions {
                store: None,
                orch_home: Some(home),
            },
        )
        .unwrap();
        assert_eq!(l.store.as_ref(), Some(&store));
        let root = std::fs::canonicalize(d.path()).unwrap();
        let pdf = std::fs::canonicalize(d.path().join("pdfs")).unwrap();
        assert_eq!(l.root_path(None).unwrap(), root);
        assert_eq!(l.root_path(Some("project")).unwrap(), root);
        assert_eq!(l.root_path(Some("pdf")).unwrap(), pdf);
        assert_eq!(
            l.root_path(Some("unknown")).unwrap_err().kind,
            "invalid_manifest"
        );
        assert_eq!(l.document_path("paper.pdf").unwrap(), pdf.join("paper.pdf"));
        let mut a = l.artifact("inventory").unwrap().clone();
        assert_eq!(
            l.artifact_path(&a).unwrap(),
            root.join("data/literature_inventory.csv")
        );
        a.root = Some("pdf".into());
        a.path = "../escape".into();
        assert_eq!(l.artifact_path(&a).unwrap_err().kind, "escape");
        assert_eq!(l.document_path("../escape").unwrap_err().kind, "escape");
        assert_eq!(l.roots(), vec![root, pdf, store]);
        assert_eq!(l.by_role(Role::Inventory).len(), 1);
        assert!(l.has(Capability::Literature));
        assert!(!l.has(Capability::PdfPages));
        std::fs::write(d.path().join(MANIFEST_FILE), r#"{"manifest_version":1,"research_id":"valid","research_version":"1","title":"T","settings":[],"versions":{}}"#).unwrap();
        assert_eq!(
            locate(d.path(), &Default::default()).unwrap_err().kind,
            "invalid_manifest"
        );
    }
}
