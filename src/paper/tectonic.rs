use crate::base::Res;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tectonic {
    pub argv: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CompileReport {
    pub pdf: Option<PathBuf>,
    pub log: PathBuf,
    pub ok: bool,
    pub unresolved_citations: Vec<String>,
    pub undefined_refs: Vec<String>,
    pub missing_files: Vec<String>,
    pub missing_chars: Vec<String>,
}
pub fn find(_data_home: &Path) -> Res<Tectonic> {
    todo!()
}
pub fn find_for_tests() -> Option<Tectonic> {
    todo!()
}
pub fn compile(
    _t: &Tectonic,
    _main: &Path,
    _offline: bool,
    _timeout: Duration,
) -> Res<CompileReport> {
    todo!()
}
pub fn compile_figure(
    _t: &Tectonic,
    _tex: &Path,
    _offline: bool,
    _timeout: Duration,
) -> Res<PathBuf> {
    todo!()
}
