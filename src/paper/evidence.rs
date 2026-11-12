use super::model::EvidenceBundle;
use crate::base::Res;
use std::path::{Path, PathBuf};

pub struct BundleInputs {
    pub store: PathBuf,
    pub project_root: PathBuf,
    pub results: Vec<PathBuf>,
    pub datasets: Vec<PathBuf>,
    pub models: Vec<PathBuf>,
}

pub fn build(_inputs: &BundleInputs) -> Res<EvidenceBundle> {
    unimplemented!()
}
pub fn save(_store: &Path, _bundle: &EvidenceBundle) -> Res<()> {
    unimplemented!()
}
pub fn load(_store: &Path) -> Res<Option<EvidenceBundle>> {
    unimplemented!()
}
pub fn verify(_bundle: &EvidenceBundle, _project_root: &Path) -> Res<()> {
    unimplemented!()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataTable {
    pub columns: Vec<String>,
    pub units: Vec<Option<String>>,
    pub rows: Vec<Vec<String>>,
}

pub fn load_table(
    _bundle: &EvidenceBundle,
    _project_root: &Path,
    _artifact: &str,
) -> Res<DataTable> {
    unimplemented!()
}
