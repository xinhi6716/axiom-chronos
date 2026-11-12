//! jcr; owned by module paper-jcr.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JcrRecord {
    pub title: String,
    pub issn: Option<String>,
    pub eissn: Option<String>,
    pub year: i32,
    pub category: String,
    pub quartile: crate::paper::model::Quartile,
    pub jif: Option<f64>,
    pub row: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JcrImport {
    pub file_name: String,
    pub file_sha256: String,
    pub imported_at: String,
    pub records: Vec<JcrRecord>,
    pub warnings: Vec<String>,
}

#[allow(unused_variables)]
pub fn import(path: &std::path::Path) -> crate::base::Res<JcrImport> {
    todo!()
}

#[allow(unused_variables)]
pub fn save(data_home: &std::path::Path, import: &JcrImport) -> crate::base::Res<()> {
    todo!()
}

#[allow(unused_variables)]
pub fn load_latest(data_home: &std::path::Path) -> crate::base::Res<Option<JcrImport>> {
    todo!()
}

impl JcrImport {
    #[allow(unused_variables)]
    pub fn quartiles_for(
        &self,
        issns: &[String],
        year: Option<i32>,
    ) -> Vec<crate::paper::model::Sourced<crate::paper::model::QuartileValue>> {
        todo!()
    }
}
