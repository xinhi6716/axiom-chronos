//! readiness; owned by module paper-review.
use super::{
    citations::CitationReport, figures::pages::PageReport, integrity::IntegrityReport,
    tectonic::CompileReport, Disclosure, JournalChoice, PaperState, Step,
};
use crate::base::Res;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    FourPasses,
    Citations,
    Numbers,
    Compile,
    Render,
    Disclosure,
    Review,
}
#[derive(Debug, Clone)]
pub struct ReadyStatus {
    pub ready: bool,
    pub conditions: Vec<(Condition, bool, String)>,
}
pub fn final_manuscript_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn journal_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn bibliography_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn pdf_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn compile_report_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn citation_report_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn numbers_report_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn render_report_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn disclosure_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn rules_path(_store: &Path) -> PathBuf {
    unimplemented!()
}
pub fn record_gate(
    _store: &Path,
    _step: Step,
    _output: &str,
    _ok: bool,
    _detail: Option<String>,
) -> Res<PaperState> {
    unimplemented!()
}
pub fn record_journal(_store: &Path, _choice: &JournalChoice) -> Res<()> {
    unimplemented!()
}
pub fn record_citations(_store: &Path, _report: &CitationReport, _bib_text: &str) -> Res<()> {
    unimplemented!()
}
pub fn record_numbers(_store: &Path, _report: &IntegrityReport) -> Res<()> {
    unimplemented!()
}
pub fn record_compile(_store: &Path, _report: &CompileReport, _pdf_bytes: &[u8]) -> Res<()> {
    unimplemented!()
}
pub fn record_render(_store: &Path, _report: &PageReport) -> Res<()> {
    unimplemented!()
}
pub fn record_disclosure(_store: &Path, _disclosure: &Disclosure) -> Res<()> {
    unimplemented!()
}
pub fn evaluate(_store: &Path, _project_root: &Path) -> Res<ReadyStatus> {
    unimplemented!()
}
pub fn mark_ready(_store: &Path, _project_root: &Path) -> Res<ReadyStatus> {
    unimplemented!()
}
pub fn is_ready(_store: &Path, _project_root: &Path) -> Res<bool> {
    unimplemented!()
}
