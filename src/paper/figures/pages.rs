use crate::{base::Res, paper::figures::audit::AuditIssue};
use serde::{Deserialize, Serialize};
use std::{ops::RangeInclusive, path::Path};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageReport {
    pub pages: u32,
    pub issues: Vec<AuditIssue>,
    pub vector_figures: bool,
}
pub fn check_pdf(
    _pdf: &Path,
    _log: &Path,
    _expect_pages: Option<RangeInclusive<u32>>,
) -> Res<PageReport> {
    todo!()
}
